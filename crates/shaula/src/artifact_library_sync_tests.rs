use super::*;

#[tokio::test]
async fn bundled_catalog_imports_and_recovers_proxmox_render_sources_from_database() -> TestResult {
    let temp = tempfile::tempdir()?;
    let bundled_templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let defaults = temp.path().join("defaults");
    // Match the installed default catalog; repository-only setup-info helpers
    // are not template sources and are not installed by the package.
    for key in ["docker", "kubernetes", "proxmox"] {
        let directory = defaults.join(key);
        std::fs::create_dir_all(&directory)?;
        let (bytes, _) = import::package(&bundled_templates.join(key))
            .map_err(|error| format!("package bundled {key}: {error}"))?;
        shaula_template::artifact::extract_tar_gz(&bytes, &directory, MAX_EXPANSION)
            .map_err(|error| format!("extract bundled {key}: {error}"))?;
    }
    let (store, library) = adapter(temp.path()).await?;
    library
        .initialize(std::slice::from_ref(&defaults), 1)
        .await
        .map_err(|error| format!("initialize bundled catalog: {error}"))?;
    let sources = store.template_sources().await?;
    assert_eq!(library.sources().await?, sources);
    assert_eq!(
        sources
            .iter()
            .map(|source| source.key.as_str())
            .collect::<Vec<_>>(),
        vec!["docker", "kubernetes", "proxmox"]
    );
    assert_eq!(store.artifact_archive_digests().await?.len(), 3);
    let proxmox = sources
        .iter()
        .find(|source| source.key == "proxmox")
        .ok_or("proxmox source missing")?;
    let archive = store
        .artifact_archive_get(&proxmox.artifact_digest)
        .await?
        .ok_or("proxmox archive missing")?;
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &proxmox.artifact_digest)
            .ok_or("bad digest")?;

    // Lose both expanded files and their local archive. Recovery has only the
    // persisted database bytes; it does not reread a default source directory.
    std::fs::remove_dir_all(&cached)?;
    std::fs::remove_file(cached.with_extension("tar.gz"))?;
    assert!(library
        .ensure_cached(&proxmox.artifact_digest)
        .await
        .map_err(|error| format!("recover proxmox cache: {error}"))?);
    assert_eq!(std::fs::read(cached.with_extension("tar.gz"))?, archive);
    for file in ["bootstrap.tftpl", "runner-service.tftpl", "user-data.tftpl"] {
        assert_eq!(
            std::fs::read(cached.join(file))?,
            std::fs::read(bundled_templates.join("proxmox").join(file))?
        );
    }
    let variables = library
        .variables(&proxmox.artifact_digest)
        .await
        .map_err(|error| format!("discover proxmox variables: {error}"))?
        .ok_or("proxmox variables missing")?;
    assert!(variables.available, "{:?}", variables.reason);
    assert_eq!(variables.artifact_digest, proxmox.artifact_digest);
    assert!(variables
        .bindings
        .iter()
        .any(|variable| variable.key == "proxmox_host"));
    Ok(())
}

#[tokio::test]
async fn root_render_templates_survive_database_recovery_and_change_source_digest() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let roots = std::slice::from_ref(&defaults);
    let render_source = defaults.join("docker/bootstrap.tftpl");
    let original_script = "#!/bin/sh\nprintf '%s' '${runner_name}'\n";
    std::fs::write(&render_source, original_script)?;
    let (store, library) = adapter(temp.path()).await?;
    library.initialize(roots, 1).await?;
    let original = library.sources().await?.remove(0);
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &original.artifact_digest)
            .ok_or("bad digest")?;
    assert_eq!(
        std::fs::read_to_string(cached.join("bootstrap.tftpl"))?,
        original_script
    );

    std::fs::remove_dir_all(&cached)?;
    assert!(library.ensure_cached(&original.artifact_digest).await?);
    assert_eq!(
        std::fs::read_to_string(cached.join("bootstrap.tftpl"))?,
        original_script
    );
    std::fs::write(&render_source, "#!/bin/sh\nexit 0\n")?;
    library.initialize(roots, 2).await?;
    let updated = library.sources().await?.remove(0);
    assert_ne!(updated.artifact_digest, original.artifact_digest);
    assert_eq!(store.artifact_archive_digests().await?.len(), 2);
    assert_eq!(
        std::fs::read_to_string(cached.join("bootstrap.tftpl"))?,
        original_script
    );
    Ok(())
}

#[tokio::test]
async fn package_upgrade_replaces_catalog_but_old_archives_still_recover() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let roots = std::slice::from_ref(&defaults);
    let (store, library) = adapter(temp.path()).await?;
    library.initialize(roots, 1).await?;
    let original = library.sources().await?.remove(0);
    let bytes = store
        .artifact_archive_get(&original.artifact_digest)
        .await?;
    assert!(library
        .variables(&original.artifact_digest)
        .await?
        .is_some_and(|v| v.available));
    let plane = SqliteControlPlane::new(store.clone(), library.root.clone())
        .with_artifact_cache(library.clone());

    // A withdrawn versioned alias must not remain in the starting-point catalog.
    let mut alias = original.clone();
    alias.key = "docker-official-2-337-0".into();
    store
        .template_sources_replace(&[original.clone(), alias], 1)
        .await?;
    let source_file = defaults.join("docker/main.tf");
    let updated = format!(
        "{}\n# Updated bundled template\n",
        std::fs::read_to_string(&source_file)?
    );
    std::fs::write(&source_file, updated)?;
    library.initialize(roots, 2).await?;
    let sources = library.sources().await?;
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].key, "docker");
    assert_ne!(sources[0].artifact_digest, original.artifact_digest);
    library.initialize(roots, 3).await?;
    assert_eq!(library.sources().await?, sources);
    assert_eq!(store.artifact_archive_digests().await?.len(), 2);
    assert!(plane.template_profile_keys().await?.is_empty());

    library.initialize(&[], 4).await?;
    assert!(library.sources().await?.is_empty());
    assert_eq!(
        store
            .artifact_archive_get(&original.artifact_digest)
            .await?,
        bytes
    );
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &original.artifact_digest)
            .ok_or("bad digest")?;
    std::fs::remove_dir_all(&cached)?;
    assert!(plane
        .artifact_parameter_schema(&original.artifact_digest)
        .await?
        .contains("runner_image"));
    assert!(cached.join("main.tf").is_file());
    Ok(())
}

#[tokio::test]
async fn all_configured_roots_sync_together_and_empty_root_removes_sources() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let second_root = temp.path().join("second");
    let second = fixture_source(&second_root)?;
    std::fs::rename(second.join("docker"), second.join("other-docker"))?;
    let (_store, library) = adapter(temp.path()).await?;
    library.initialize(&[defaults.clone(), second], 1).await?;
    assert_eq!(
        library
            .sources()
            .await?
            .iter()
            .map(|s| s.key.as_str())
            .collect::<Vec<_>>(),
        vec!["docker", "other-docker"]
    );
    std::fs::remove_dir_all(defaults.join("docker"))?;
    library.initialize(&[defaults], 2).await?;
    assert!(library.sources().await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn missing_roots_and_duplicate_keys_preserve_previous_catalog() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let (_store, library) = adapter(temp.path()).await?;
    library
        .initialize(std::slice::from_ref(&defaults), 1)
        .await?;
    let original = library.sources().await?;
    assert!(library
        .initialize(&[defaults.clone(), temp.path().join("missing")], 2)
        .await
        .is_err());
    assert_eq!(library.sources().await?, original);
    let duplicate = fixture_source(&temp.path().join("second"))?;
    assert!(library.initialize(&[defaults, duplicate], 3).await.is_err());
    assert_eq!(library.sources().await?, original);
    Ok(())
}

#[tokio::test]
async fn one_invalid_source_does_not_publish_an_earlier_valid_update() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let (store, library) = adapter(temp.path()).await?;
    library
        .initialize(std::slice::from_ref(&defaults), 1)
        .await?;
    let original = library.sources().await?;
    let file = defaults.join("docker/main.tf");
    std::fs::write(
        &file,
        format!("{}\n# valid update\n", std::fs::read_to_string(&file)?),
    )?;
    let broken = fixture_source(&temp.path().join("second"))?;
    std::fs::rename(broken.join("docker"), broken.join("z-invalid"))?;
    std::fs::write(broken.join("z-invalid/main.tf"), "invalid source")?;
    assert!(library
        .initialize(&[defaults.clone(), broken], 2)
        .await
        .is_err());
    assert_eq!(library.sources().await?, original);
    // A validated archive may be reused on retry without becoming a visible default.
    assert_eq!(store.artifact_archive_digests().await?.len(), 2);
    library.initialize(&[defaults], 3).await?;
    assert_ne!(library.sources().await?, original);
    Ok(())
}

#[tokio::test]
async fn failed_new_source_cache_write_keeps_catalog_for_retry() -> TestResult {
    let temp = tempfile::tempdir()?;
    let defaults = fixture_source(temp.path())?;
    let (_store, library) = adapter(temp.path()).await?;
    library
        .initialize(std::slice::from_ref(&defaults), 1)
        .await?;
    let original = library.sources().await?;
    let file = defaults.join("docker/main.tf");
    std::fs::write(
        &file,
        format!(
            "{}\n# cache failure update\n",
            std::fs::read_to_string(&file)?
        ),
    )?;
    let (bytes, _) = import::package(&defaults.join("docker"))?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    let cached =
        shaula_core::artifact_layout::artifact_dir(&library.root, &digest).ok_or("bad digest")?;
    std::fs::create_dir_all(cached.parent().ok_or("parent missing")?)?;
    std::fs::write(&cached, b"not a directory")?;
    assert!(library
        .initialize(std::slice::from_ref(&defaults), 2)
        .await
        .is_err());
    assert_eq!(library.sources().await?, original);
    std::fs::remove_file(&cached)?;
    library.initialize(&[defaults], 3).await?;
    assert_eq!(library.sources().await?[0].artifact_digest, digest);
    Ok(())
}
