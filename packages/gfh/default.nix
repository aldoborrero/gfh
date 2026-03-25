{ pkgs, inputs, ... }:
pkgs.callPackage ./package.nix { inherit inputs; }
