{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    rust-analyzer
    git
    gh
    glab
  ];
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
}
