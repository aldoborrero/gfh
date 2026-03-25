{
  description = "Git FIDO helper - use multiple FIDO keys to sign Git commits";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    blueprint.url = "github:numtide/blueprint";
    blueprint.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: inputs.blueprint { inherit inputs; };
}
