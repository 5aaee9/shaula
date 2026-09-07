//! Engine execution integration tests. Since R7-06 every engine spawn
//! on Windows runs behind the fence supervisor, whose hidden marker only
//! the real `shaula` binary routes — so these tests point the supervisor
//! at `CARGO_BIN_EXE_shaula` and exercise the REAL production path:
//! supervisor → kill-on-close job → whole-tree termination, plus the
//! Terraform flow protocol against fixture engines.

#![allow(clippy::unwrap_used)]

#[cfg(windows)]
mod fence {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use shaula_template::engine::{run_engine, TerraformFlow};

    /// Route the fence supervisor at the real binary (a test-harness
    /// binary cannot route the hidden marker itself).
    fn route_supervisor() {
        std::env::set_var("SHAULA_ENGINE_FENCE_EXE", env!("CARGO_BIN_EXE_shaula"));
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[tokio::test]
    async fn environment_allowlist_isolated() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // A batch script that dumps SHAULA_SECRET presence.
        let script = write_script(
            dir.path(),
            "fake-tf.cmd",
            "@echo off\r\nif defined SHAULA_SECRET (echo LEAKED) else (echo CLEAN)\r\n",
        );
        let output = run_engine(&script, dir.path(), &[], &[], Duration::from_secs(10))
            .await
            .unwrap();
        assert!(
            output.stdout.contains("CLEAN"),
            "daemon env must not leak into the child"
        );
        assert!(!output.stdout.contains("LEAKED"));

        // Declared extra env does pass through.
        let script2 = write_script(
            dir.path(),
            "fake-tf2.cmd",
            "@echo off\r\necho %TF_IN_AUTOMATION%\r\n",
        );
        let output = run_engine(&script2, dir.path(), &[], &[], Duration::from_secs(10))
            .await
            .unwrap();
        assert!(output.stdout.contains('1'));
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_invocation() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            dir.path(),
            "slow.cmd",
            // env_clear strips PATH, so call ping by absolute path through
            // the allowlisted SystemRoot.
            "@echo off\r\n\"%SystemRoot%\\System32\\ping.exe\" -n 30 127.0.0.1 > nul\r\n",
        );
        let result = run_engine(&script, dir.path(), &[], &[], Duration::from_millis(500)).await;
        assert!(result.is_err(), "long child must hit the bounded timeout");
    }

    #[tokio::test]
    async fn plan_argv_freezes_the_protected_input() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // Fixture engine: `version` answers the open() probe, `plan` echoes
        // its argv so the test can assert the exact command surface.
        let script = write_script(
            dir.path(),
            "tf-echo.cmd",
            "@echo off\r\nif \"%1\"==\"version\" (echo Terraform v9.9.9-test\r\nexit /b 0)\r\nif \"%1\"==\"plan\" (echo %*\r\nexit /b 0)\r\nexit /b 1\r\n",
        );
        let flow = TerraformFlow::open(script, Duration::from_secs(10))
            .await
            .unwrap();

        let create = flow.plan(dir.path(), &[], false).await.unwrap();
        let argv = create.stdout;
        assert!(
            argv.contains("-var-file=shaula.tfvars.json"),
            "saved plan must freeze the protected input: {argv}"
        );
        assert!(argv.contains("-out=tfplan"));
        assert!(argv.contains("-input=false"));
        assert!(!argv.contains("-destroy"));

        let destroy = flow.plan(dir.path(), &[], true).await.unwrap();
        assert!(
            destroy.stdout.contains("-var-file=shaula.tfvars.json")
                && destroy.stdout.contains("-destroy"),
            "destroy plan needs the same frozen input: {}",
            destroy.stdout
        );
    }

    #[tokio::test]
    async fn state_list_classifies_official_missing_state_diagnostic() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // Official Terraform v1.9 fresh-workspace diagnostic, matched
        // case-insensitively (T01).
        let script = write_script(
            dir.path(),
            "tf-nostate.cmd",
            "@echo off\r\nif \"%1\"==\"version\" (echo Terraform v9.9.9-test\r\nexit /b 0)\r\necho No state file was found! 1>&2\r\nexit /b 1\r\n",
        );
        let flow = TerraformFlow::open(script, Duration::from_secs(10))
            .await
            .unwrap();
        let addresses = flow.state_list(dir.path(), &[]).await.unwrap();
        assert!(
            addresses.is_empty(),
            "official missing-state diagnostic must classify as empty"
        );
    }

    #[tokio::test]
    async fn state_list_fails_closed_on_unrelated_errors() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            dir.path(),
            "tf-backend.cmd",
            "@echo off\r\nif \"%1\"==\"version\" (echo Terraform v9.9.9-test\r\nexit /b 0)\r\necho backend connection refused 1>&2\r\nexit /b 1\r\n",
        );
        let flow = TerraformFlow::open(script, Duration::from_secs(10))
            .await
            .unwrap();
        assert!(
            flow.state_list(dir.path(), &[]).await.is_err(),
            "unrelated state-list errors must fail closed, not read as empty"
        );
    }

    #[tokio::test]
    async fn state_pull_snapshot_extracts_managed_only_identity_from_raw_v4() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // RAW state v4 — the actual `terraform state pull` contract
        // (statefile/version4.go): ALL resources at the TOP LEVEL, each
        // carrying its `module` path, and per-instance `index_key` for
        // count/for_each. The plan JSON keeps these instance-level
        // addresses, so exact-state coverage compares like for like (R5-01).
        let state_json = r#"{"version":4,"terraform_version":"1.9.8","serial":42,"lineage":"lineage-abc","outputs":{},"resources":[
            {"mode":"managed","type":"kubernetes_secret_v1","name":"bootstrap","instances":[{"attributes":{"data":"x"}}]},
            {"mode":"managed","type":"docker_container","name":"runner","module":"module.runners","instances":[
                {"index_key":0,"attributes":{"id":"a"}},
                {"index_key":1,"attributes":{"id":"b"}}
            ]},
            {"mode":"managed","type":"kubernetes_pod_v1","name":"by_key","module":"module.runners","instances":[
                {"index_key":"pool-a","attributes":{"id":"c"}}
            ]},
            {"mode":"managed","type":"kubernetes_config_map_v1","name":"extra","module":"module.runners.module.nested","instances":[{"attributes":{}}]},
            {"mode":"managed","type":"kubernetes_pod_v1","name":"zoned","module":"module.runners[\"east.us\"]","instances":[{"index_key":"zone-b","attributes":{"id":"z"}}]},
            {"mode":"managed","type":"null_resource","name":"exhausted","instances":[]},
            {"mode":"data","type":"kubernetes_namespace","name":"target","instances":[{"attributes":{"id":"ns"}}]}
        ]}"#;
        // The JSON body is written as a data file and `type`d by the fixture
        // engine — cmd's echo cannot reliably carry quoted JSON.
        std::fs::write(dir.path().join("state.json"), state_json).unwrap();
        let script = write_script(
            dir.path(),
            "tf-state.cmd",
            "@echo off\r\nif \"%1\"==\"version\" (echo Terraform v9.9.9-test\r\nexit /b 0)\r\nif \"%1\"==\"state\" (type state.json\r\nexit /b 0)\r\nexit /b 1\r\n",
        );
        let flow = TerraformFlow::open(script, Duration::from_secs(10))
            .await
            .unwrap();
        let snapshot = flow.state_pull_snapshot(dir.path(), &[]).await.unwrap();
        assert_eq!(snapshot.serial, 42);
        assert_eq!(snapshot.lineage, "lineage-abc");
        // Full instance addresses with module paths and index keys; data
        // sources are NOT deletion targets; a zero-instance resource is NOT
        // a live target.
        assert_eq!(
            snapshot.managed,
            vec![
                "kubernetes_secret_v1.bootstrap".to_string(),
                "module.runners.docker_container.runner[0]".to_string(),
                "module.runners.docker_container.runner[1]".to_string(),
                "module.runners.kubernetes_pod_v1.by_key[\"pool-a\"]".to_string(),
                "module.runners.module.nested.kubernetes_config_map_v1.extra".to_string(),
                // R6-05: a dot INSIDE a quoted for_each module key is data,
                // never a separator — the hostname-style key survives.
                "module.runners[\"east.us\"].kubernetes_pod_v1.zoned[\"zone-b\"]".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn state_pull_rejects_show_json_and_corrupt_shapes() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // The `terraform show -json` shape (values.root_module) is NOT the
        // state-pull contract: reading it as empty let Destroy pass over
        // live resources (F01). It must be rejected, never parsed.
        let show_shape = r#"{"format_version":"1.2","values":{"root_module":{"resources":[
            {"mode":"managed","type":"kubernetes_pod_v1","name":"runner","address":"kubernetes_pod_v1.runner"}
        ]}}}"#;
        // Raw v4 without a lineage cannot prove ownership.
        let no_lineage = r#"{"version":4,"serial":7,"resources":[]}"#;
        // An unknown resource mode is corrupt, not ignorable.
        let unknown_mode = r#"{"version":4,"serial":7,"lineage":"l","resources":[
            {"mode":"synced","type":"x","name":"y","instances":[]}
        ]}"#;
        // A malformed module path is corrupt, not a root resource.
        let bad_module = r#"{"version":4,"serial":7,"lineage":"l","resources":[
            {"mode":"managed","type":"x","name":"y","module":"runners","instances":[{"index_key":0}]}
        ]}"#;
        // A missing instances array is corrupt, not empty.
        let no_instances = r#"{"version":4,"serial":7,"lineage":"l","resources":[
            {"mode":"managed","type":"x","name":"y","module":"module.a"}
        ]}"#;
        for (name, body) in [
            ("show.json", show_shape),
            ("nolineage.json", no_lineage),
            ("badmode.json", unknown_mode),
            ("badmodule.json", bad_module),
            ("noinstances.json", no_instances),
        ] {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        let script = write_script(
            dir.path(),
            "tf-state2.cmd",
            "@echo off\r\nif \"%1\"==\"version\" (echo Terraform v9.9.9-test\r\nexit /b 0)\r\nif \"%1\"==\"state\" (if \"%2\"==\"pull\" (type %SH_STATE%\r\nexit /b 0))\r\nexit /b 1\r\n",
        );
        let flow = TerraformFlow::open(script, Duration::from_secs(10))
            .await
            .unwrap();
        // EVERY corrupt-shape fixture above is executed through the real
        // engine path: each must be rejected, never read as an empty state.
        for name in [
            "show.json",
            "nolineage.json",
            "badmode.json",
            "badmodule.json",
            "noinstances.json",
        ] {
            // SH_STATE is delivered through the DECLARED extra-env channel
            // (the child env is cleared), so the failure below can only come
            // from the parser rejecting the document.
            let env = vec![("SH_STATE".to_string(), name.to_string())];
            let result = flow.state_pull_snapshot(dir.path(), &env).await;
            assert!(
                result.is_err(),
                "state document {name} must be rejected, not read as empty"
            );
        }
    }

    #[tokio::test]
    async fn cancelling_run_engine_kills_the_child_no_late_effects() {
        route_supervisor();
        let dir = tempfile::tempdir().unwrap();
        // The child announces it started, then (much later) performs a
        // side effect. A cancelled run_engine must never let it land.
        let script = write_script(
            dir.path(),
            "late.cmd",
            r#"@echo off
echo started > started.txt
"%SystemRoot%\System32\ping.exe" -n 6 127.0.0.1 > nul
echo late > late-effect.txt
"#,
        );
        // Spawn the CALLER like the runtime does, then cancel it: the
        // abort drops the run_engine future and with it the supervisor
        // child, whose job close terminates the engine.
        let task = tokio::spawn({
            let script = script.clone();
            let cwd = dir.path().to_path_buf();
            async move { run_engine(&script, &cwd, &[], &[], Duration::from_secs(30)).await }
        });

        // Wait until the child actually started (marker on disk), then
        // cancel the caller exactly like a supervisor shutdown would.
        let started = dir.path().join("started.txt");
        for _ in 0..50 {
            if started.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(started.exists(), "fixture child must have started");
        task.abort();
        let _ = task.await;

        // Give any SURVIVING child ample time to write the late effect. It
        // must never appear: the fence closed at cancellation.
        let late = dir.path().join("late-effect.txt");
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !late.exists(),
                "a cancelled run_engine must not leave a live child behind"
            );
        }
    }

    #[tokio::test]
    async fn cancelled_run_engine_terminates_descendants_no_late_marker() {
        route_supervisor();
        // R6-04/R7-06: a DESCENDANT spawned by the engine must die with
        // the fence — the child spawns a detached grandchild that would
        // write a marker ~8s later; after the caller is cancelled, that
        // marker must never appear. The grandchild is a BORN member of
        // the supervisor's kill-on-close job, so there is no escape.
        let dir = tempfile::tempdir().unwrap();
        let descendant = dir.path().join("descendant-marker.txt");
        let descendant = descendant.to_string_lossy().into_owned();
        let script = write_script(
            dir.path(),
            "spawn-child.cmd",
            &format!(
                r#"@echo off
echo armed > armed.txt
start "" /b cmd /c "ping -n 8 127.0.0.1 > nul & echo descendant > {descendant}"
"#
            ),
        );
        let task = tokio::spawn({
            let script = script.clone();
            let cwd = dir.path().to_path_buf();
            async move { run_engine(&script, &cwd, &[], &[], Duration::from_secs(30)).await }
        });

        // Wait until the child actually SPAWNED the grandchild (arm
        // marker) so the test cannot pass vacuously.
        let armed = dir.path().join("armed.txt");
        for _ in 0..100 {
            if armed.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(armed.exists(), "fixture child must have armed");
        // Give the `start` a moment to create the grandchild process.
        tokio::time::sleep(Duration::from_millis(300)).await;
        task.abort();
        let _ = task.await;

        // The grandchild would write its marker at ~8s; it must never land.
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                !dir.path().join("descendant-marker.txt").exists(),
                "the process-tree fence must terminate descendants (R6-04/R7-06)"
            );
        }
    }
}
