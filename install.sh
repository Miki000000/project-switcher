#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${PROJ_BIN_DIR:-$HOME/.local/bin}"
REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

info() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

cd "$REPO_DIR"

command -v cargo >/dev/null 2>&1 || die "cargo is not installed. Install Rust (https://rustup.rs) and run this script again."
info "Building with $(cargo --version)"
cargo build --release --locked

BINARY="$REPO_DIR/target/release/proj"
[ -x "$BINARY" ] || die "build finished but $BINARY was not produced"

mkdir -p "$BIN_DIR"
if [ -e "$BIN_DIR/proj" ] || [ -L "$BIN_DIR/proj" ]; then
  info "Removing previous $BIN_DIR/proj"
  rm -f -- "$BIN_DIR/proj"
fi
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

command -v zsh >/dev/null 2>&1 || warn "zsh not found; changing directory on selection currently works only in zsh"

rc="${ZDOTDIR:-$HOME}/.zshrc"
hook="command -v proj >/dev/null && eval \"\$(proj init zsh)\""

echo
if [ -f "$rc" ] && grep -q "proj init zsh" "$rc"; then
  info "Shell integration already present in $rc"
elif { [ -L "$rc" ] && [[ "$(readlink -f -- "$rc")" == /nix/store/* ]]; } || { [ -e "$rc" ] && [ ! -w "$rc" ]; }; then
  warn "$rc is managed by Nix (Home Manager) and cannot be edited by this script."
  cat <<EOF
Add this to your Home Manager configuration and switch, so proj can change your directory:

    programs.zsh.initContent = ''$hook'';

EOF
else
  printf '\n%s\n' "$hook" >>"$rc"
  info "Added shell integration to $rc"
fi

echo "Open a new zsh terminal and run: proj"
