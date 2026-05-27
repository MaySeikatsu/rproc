{
  description = "Resource & process monitor for Linux, inspired by Windows 11 Task Manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        buildInputs = with pkgs; [
          # OpenGL (eframe glow backend)
          libGL

          # X11 (eframe x11 feature)
          xorg.libX11
          xorg.libxcb
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr

          # Wayland (eframe wayland feature)
          wayland
          libxkbcommon
        ];

        runtimeLibPath = pkgs.lib.makeLibraryPath buildInputs;
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "rproc";
          version = "0.1.3";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          inherit nativeBuildInputs buildInputs;

          postFixup = ''
            patchelf --add-rpath ${runtimeLibPath} $out/bin/rproc
          '';
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/rproc";
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
