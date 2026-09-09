{
  lib,
  makeRustPlatform,
  fetchNpmDeps,
  nodejs,
  npmHooks,
  pkg-config,
  cargo-nextest,
  terraform,
  toolchain,
}:
let
  src = import ./source.nix { inherit lib; };
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
in
rustPlatform.buildRustPackage {
  pname = "shaula";
  version = (lib.importTOML ../../Cargo.toml).workspace.package.version;
  inherit src;

  cargoLock.lockFile = ../../Cargo.lock;
  cargoBuildFlags = [ "--workspace" ];
  # Standard-library panic locations must not retain the entire fenix toolchain.
  RUSTFLAGS = "--remap-path-prefix=${toolchain}=/rust-toolchain";
  disallowedReferences = [
    toolchain
    nodejs
  ];

  # build.rs runs Vite itself, so provide exactly the locked npm tree offline.
  # Node and node_modules are build inputs, never runtime dependencies.
  npmDeps = fetchNpmDeps {
    src = lib.cleanSource ../../web;
    hash = "sha256-/fkAcd9+NTYVskgVz/J1udS0K0C+EM+kHAJ9UrkY17w=";
  };
  npmRoot = "web";
  nativeBuildInputs = [
    pkg-config
    nodejs
    npmHooks.npmConfigHook
  ];
  nativeCheckInputs = [
    cargo-nextest
    terraform
  ];
  NODE = lib.getExe nodejs;

  preBuild = ''
    npm exec --prefix web -- tsc --noEmit --project web/tsconfig.json
  '';
  postInstall = ''
    for name in docker kubernetes proxmox; do
      destination="$out/share/shaula/templates/$name"
      mkdir -p "$destination"
      cp "templates/$name/profile.yaml" \
        "templates/$name/.terraform.lock.hcl" "$destination/"
      for source in "templates/$name"/*.tf "templates/$name"/*.tf.json "templates/$name"/*.tftpl; do
        if test -f "$source"; then
          cp "$source" "$destination/"
        fi
      done
      cp -R "templates/$name/schemas" "$destination/"
      if test -f "templates/$name/runtime-policy.md"; then
        cp "templates/$name/runtime-policy.md" "$destination/"
      fi
    done
  '';
  # Local-server test constructors deliberately do not exist in release
  # builds. Test in debug without enabling those seams in the shipped binary.
  # nextest's run output never reaches the streamed Nix build log, so the run
  # goes through a file: echoed back on failure to name the failing test, and
  # summarized on success so bounded retries keep rare hosted-runner flakes
  # visible instead of silently masking them.
  checkPhase = ''
    runHook preCheck
    cargo clippy --offline --locked --workspace --all-targets -- -D warnings
    if cargo nextest run --offline --locked --manifest-path Cargo.toml --workspace \
      --retries 2 >nextest-output.log 2>&1; then
      tail -n 40 nextest-output.log
    else
      status=$?
      cat nextest-output.log
      exit "$status"
    fi
    SHAULA_TEST_TERRAFORM=${lib.getExe terraform} \
      cargo test --offline --locked --workspace --test http_state_backend -- --include-ignored
    runHook postCheck
  '';

  meta = {
    description = "GitHub Actions Scale Set capacity controller with an embedded operator UI";
    homepage = "https://github.com/5aaee9/shaula";
    license = lib.licenses.asl20;
    mainProgram = "shaula";
    platforms = lib.platforms.linux;
  };
}
