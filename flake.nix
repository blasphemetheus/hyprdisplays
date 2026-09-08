{
  description = "hyprdisplays — display/monitor manager for Hyprland (Lua config era)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      # Runtime libs a winit/wgpu app dlopen()s: Wayland + xkb for the window,
      # Vulkan loader for wgpu (the NVIDIA ICD comes from /run/opengl-driver).
      runtimeLibs = with pkgs; [ wayland libxkbcommon vulkan-loader libGL ];
    in {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "hyprdisplays";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
        buildInputs = runtimeLibs;
        postInstall = ''
          wrapProgram $out/bin/hyprdisplays \
            --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib"
          install -Dm644 assets/hyprdisplays.desktop $out/share/applications/hyprdisplays.desktop
        '';
        meta.mainProgram = "hyprdisplays";
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [ rustc cargo rust-analyzer clippy rustfmt pkg-config ] ++ runtimeLibs;
        LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib";
        RUST_BACKTRACE = "1";
      };
    };
}
