{
  description = "INZI";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        libraries = with pkgs; [
          webkitgtk_4_1  # Tauri v2는 4.1을 주로 사용
          gtk3           # 윈도우 UI 프레임워크
          cairo          # 그래픽 드로잉
          gdk-pixbuf     # 이미지 처리
          glib           # 기본 라이브러리
          dbus           # 프로세스 간 통신
          openssl        # 보안 연결 (ssl)
          librsvg        # 아이콘 등 SVG 처리
          libsoup_3      # HTTP 클라이언트
        ];

        packages = with pkgs; [
          # Rust
          rustToolchain

          # Frontend (Tauri는 Node.js 필수)
          nodejs_24      # 최신 LTS 버전 권장
          nodePackages.pnpm
          typescript
          typescript-language-server

          # Tauri CLI
          cargo-tauri    # cargo tauri 명령어
          cargo-xwin
          clang
          llvmPackages.libclang

          # System Build Tools
          pkg-config     # 라이브러리 찾기 필수 도구
          wget
          curl
          lld
        ] ++ libraries;
        # Rust 툴체인 설정 (Stable 버전 + rust-analyzer 포함)
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "i686-pc-windows-msvc" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {

          buildInputs = packages;

          CC_i686_pc_windows_msvc = "${pkgs.clang}/bin/clang";
          CXX_i686_pc_windows_msvc = "${pkgs.clang}/bin/clang++";
          AR_i686_pc_windows_msvc = "${pkgs.llvmPackages.llvm}/bin/llvm-ar";
          RC_i686_pc_windows_msvc = "${pkgs.llvmPackages.llvm}/bin/llvm-rc";

          CRATE_CC_NO_DEFAULTS = "1";

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}:${./src-tauri/lib}";
          LIBCLANG_PATH = pkgs.lib.makeLibraryPath [ pkgs.llvmPackages.libclang.lib ];

          shellHook = ''
            export PATH="${pkgs.llvmPackages.llvm}/bin:$PATH"
            export XWIN_ARCH="x86"
            export XWIN_CACHE_DIR="$HOME/.cache/xwin"
            export CFLAGS_i686_pc_windows_msvc="-target i686-pc-windows-msvc -Wno-unused-command-line-argument -I$XWIN_CACHE_DIR/xwin/crt/include -I$XWIN_CACHE_DIR/xwin/sdk/include/ucrt -I$XWIN_CACHE_DIR/xwin/sdk/include/um -I$XWIN_CACHE_DIR/xwin/sdk/include/shared"
            export LIBSQLITE3_SYS_BUNDLING=1

            export RUSTFLAGS="-L native=$XWIN_CACHE_DIR/xwin/crt/lib/x86 \
                              -L native=$XWIN_CACHE_DIR/xwin/sdk/lib/ucrt/x86 \
                              -L native=$XWIN_CACHE_DIR/xwin/sdk/lib/um/x86"
            export CFLAGS_i686_pc_windows_msvc="-Wno-unused-command-line-argument"

            export CC_i686_pc_windows_msvc="${pkgs.clang}/bin/clang"
            export CXX_i686_pc_windows_msvc="${pkgs.clang}/bin/clang++"
            export AR_i686_pc_windows_msvc="${pkgs.llvmPackages.llvm}/bin/llvm-lib"

            echo "✅ Windows 32bit (i686-pc-windows-msvc) build environment loaded."
            echo "💡 Run 'cargo xwin build --target i686-pc-windows-msvc' to compile."export PKG_CONFIG_ALLOW_CROSS=1
          '';
        };
      }
    );
}
