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
          stdenv.cc.cc.lib
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

          # System Build Tools
          pkg-config     # 라이브러리 찾기 필수 도구
          dbus           # dbus-daemon 등
          wget
          curl
        ] ++ libraries;
        # Rust 툴체인 설정 (Stable 버전 + rust-analyzer 포함)
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          # 1. 사용할 패키지 목록
          buildInputs = packages;

          # 2. 환경 변수 설정
          # rust-analyzer가 표준 라이브러리 소스를 찾을 수 있도록 경로 지정
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath libraries}:${./src-tauri/lib}";
          XDG_DATA_DIRS = "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS";
        };
      }
    );
}
