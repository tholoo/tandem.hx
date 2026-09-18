{
  description = "Tandem: human-controlled implementation proposals";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      eachSystem = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in {
      packages = eachSystem (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "tandem";
            version = "0.1.0";
            src = pkgs.lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.git ];
            postInstall = ''
              wrapProgram $out/bin/tandem --set-default TANDEM_BWRAP ${pkgs.bubblewrap}/bin/bwrap --suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.git pkgs.bubblewrap ]}
              mkdir -p $out/share/tandem
              cp -r helix $out/share/tandem/
            '';
            meta = { description = "Human-controlled coding proposals inside Helix"; license = pkgs.lib.licenses.mit; mainProgram = "tandem"; platforms = systems; };
          };
        });
      apps = eachSystem (system: {
        default = { type = "app"; program = "${self.packages.${system}.default}/bin/tandem"; };
      });
      checks = eachSystem (system: { tandem = self.packages.${system}.default; });
      devShells = eachSystem (system:
        let pkgs = pkgsFor system; in {
          default = pkgs.mkShell {
            packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer git bubblewrap (python3.withPackages (ps: [ ps.pyte ])) ];
            TANDEM_BWRAP = "${pkgs.bubblewrap}/bin/bwrap";
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        });
    };
}
