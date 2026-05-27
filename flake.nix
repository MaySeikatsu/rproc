{
  description = "Resource & process monitor for Linux, inspired by Windows 11 Task Manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        buildInputs = with pkgs; [
          # OpenGL (eframe glow backend)
          libGL

          # X11 (eframe x11 feature)
          libx11
          libxcb
          libxcursor
          libxi
          libxrandr

          # Wayland (eframe wayland feature)
          wayland
          libxkbcommon
        ];

        runtimeLibPath = pkgs.lib.makeLibraryPath buildInputs;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          inherit nativeBuildInputs buildInputs;
        };

        # Build deps-only crate for layer caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        rproc = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          postFixup = ''
            patchelf --add-rpath ${runtimeLibPath} $out/bin/rproc
          '';
        });
      in
      {
        packages.default = rproc;

        apps.default = {
          type = "app";
          program = "${rproc}/bin/rproc";
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = nativeBuildInputs ++ (with pkgs; [
            rustc
            cargo
            rust-analyzer
            clippy
            rustfmt
          ]);

          inherit buildInputs;

          LD_LIBRARY_PATH = runtimeLibPath;
        };
      }
    );
}
