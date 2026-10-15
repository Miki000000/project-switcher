#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${PROJ_BIN_DIR:-$HOME/.local/bin}"
REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
BUILD_CMD="cargo build --release --locked"

info() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

cd "$REPO_DIR"

if command -v nix-shell >/dev/null 2>&1; then
  info "Nix found: building inside nix-shell"
  nix-shell "$REPO_DIR/shell.nix" --run "$BUILD_CMD"
elif command -v cargo >/dev/null 2>&1; then
  info "Nix not found: building with the system cargo"
  $BUILD_CMD
else
  die "neither nix nor cargo is installed. Install Nix (https://nixos.org/download) or Rust (https://rustup.rs) and run this script again."
fi

BINARY="$REPO_DIR/target/release/proj"
[ -x "$BINARY" ] || die "build finished but $BINARY was not produced"

mkdir -p "$BIN_DIR"
install -m 755 "$BINARY" "$BIN_DIR/proj"
info "Installed $("$BIN_DIR/proj" --version) to $BIN_DIR/proj"

command -v git >/dev/null 2>&1 || warn "git is not on PATH; proj needs it at runtime"

case ":$PATH:" in
  *":$BIN_DIR:"*)
    resolved="$(command -v proj || true)"
    if [ -n "$resolved" ] && [ "$resolved" != "$BIN_DIR/proj" ]; then
      warn "another proj at $resolved comes first on PATH"
    fi
    ;;
  *) warn "$BIN_DIR is not on PATH; add it to your shell config: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

case "$(basename -- "${SHELL:-}")" in
  fish) hook="proj init fish | source" rc="$HOME/.config/fish/config.fish" ;;
  bash) hook="eval \"\$(proj init bash)\"" rc="$HOME/.bashrc" ;;
  *) hook="eval \"\$(proj init zsh)\"" rc="$HOME/.zshrc" ;;
esac
cat <<EOF

To let proj change your directory when you pick a repository, add this to $rc:

    $hook

Then open a new terminal and run: proj
EOF
