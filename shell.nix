{ pkgs ? import <nixpkgs> { } }:
with pkgs;
mkShell { buildInputs = [ cargo clippy rustfmt ]; }
