{
  lib,
  stdenvNoCC,
  fetchurl,
  unzip,
}:
let
  archives = {
    x86_64-linux = {
      arch = "amd64";
      sha256 = "186e0145f5e5f2eb97cbd785bc78f21bae4ef15119349f6ad4fa535b83b10df8";
    };
    aarch64-linux = {
      arch = "arm64";
      sha256 = "f85868798834558239f6148834884008f2722548f84034c9b0f62934b2d73ebb";
    };
  };
  archive = archives.${stdenvNoCC.hostPlatform.system};
in
# Match the existing protocol acceptance pin. These are the official release
# ZIP digests from https://releases.hashicorp.com/terraform/1.9.8/terraform_1.9.8_SHA256SUMS.
# Do not strip/rewrite the binary: its exact bytes are attestation authority.
stdenvNoCC.mkDerivation rec {
  pname = "terraform";
  version = "1.9.8";
  src = fetchurl {
    url = "https://releases.hashicorp.com/terraform/${version}/terraform_${version}_linux_${archive.arch}.zip";
    inherit (archive) sha256;
  };
  nativeBuildInputs = [ unzip ];
  dontUnpack = true;
  dontFixup = true;
  installPhase = ''
    runHook preInstall
    unzip "$src"
    install -Dm755 terraform "$out/bin/terraform"
    install -Dm644 LICENSE.txt "$out/share/licenses/terraform/LICENSE.txt"
    runHook postInstall
  '';
  meta = {
    description = "Pinned Terraform CLI for Shaula protocol acceptance";
    homepage = "https://www.terraform.io/";
    license = lib.licenses.bsl11;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    mainProgram = "terraform";
    platforms = builtins.attrNames archives;
  };
}
