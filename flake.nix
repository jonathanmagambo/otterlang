{
  description = "otterlang flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];

        pkgs = import nixpkgs {
          inherit system overlays;
        };

        hasInfix = pkgs.lib.strings.hasInfix;
      in
      {
        devShells.default =
          with pkgs;
          let
            llvmPackages = pkgs.llvmPackages_17;
            llvm_bin = llvmPackages.llvm;
            llvm_libs = llvmPackages.libllvm;
            rustToolchain = rust-bin.nightly.latest.default.override {
              extensions = [
                "rust-src"
                "rustc-codegen-cranelift-preview"
              ];
            };
          in
          mkShell {
            buildInputs = [
              rustToolchain
              llvm_bin
              llvm_libs
            ]
            ++ lib.optionals (hasInfix "linux" system) [
              mold
            ];

            shellHook = ''
              export PATH="${rustToolchain}/bin:$PATH"
            '';

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            LLVM_SYS_170_PREFIX = "${llvm_bin}";
            LD_LIBRARY_PATH = lib.optionalString (hasInfix "linux" system) "${llvm_bin}/lib:${llvm_libs}/lib";
            LIBRARY_PATH = lib.optionalString (hasInfix "linux" system) "${llvm_bin}/lib:${llvm_libs}/lib";

            RUSTFLAGS =
              "-Zshare-generics=y" + lib.optionalString (hasInfix "linux" system) " -Clink-arg=-fuse-ld=mold";

            CARGO_PROFILE_DEV_CODEGEN_BACKEND = (if hasInfix "linux" system then "cranelift" else "llvm");
            CARGO_NET_GIT_FETCH_WITH_CLI = "true";
          };
      }
    );
}
