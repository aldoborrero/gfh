{ pkgs, flake, ... }:
pkgs.callPackage ./package.nix { inherit pkgs flake; }
