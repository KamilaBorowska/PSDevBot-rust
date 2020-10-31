{ pkgs ? import <nixpkgs> { } }:
with pkgs;
mkShell { buildInputs = [ cargo clippy openssl pkg-config rustfmt ]; }
