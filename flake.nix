{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
  }: let
    system = "aarch64-darwin";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [rust-overlay.overlays.default];
    };
    #toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
  in {
    devShells.${system}.default = pkgs.mkShell {
      packages = [
        #toolchain
      ];

      nativeBuildInputs = [
        #pkgs.clang_19
        #pkgs.cmake
      ];

      CXXFLAGS_aarch64_apple_darwin = "--target=aarch64-apple-darwin";

    };

    packages.${system}.default = pkgs.rustPlatform.buildRustPackage rec {
      pname = manifest.name;
      inherit (manifest) version;
      cargoLock.lockFile = ./Cargo.lock;
      src = pkgs.lib.cleanSource ./.;
    };
  };
}
