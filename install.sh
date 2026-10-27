#!/usr/bin/env bash
# Instala o screenshotd (BTRP) a partir do código-fonte.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HOME/.local/bin/screenshotd"
UNIT="$HOME/.config/systemd/user/screenshotd.service"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "erro: '$1' não encontrado. Instale: $2" >&2
    exit 1
  }
}

need cargo "cargo/rustc"
need grim "grim"
need wl-copy "wl-clipboard"
need systemctl "systemd"

echo "==> compilando (release)"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

echo "==> instalando binário em $BIN"
install -Dm755 "$ROOT/target/release/screenshotd" "$BIN"

echo "==> instalando unit em $UNIT"
install -Dm644 "$ROOT/systemd/screenshotd.service" "$UNIT"

echo "==> habilitando serviço de usuário"
systemctl --user daemon-reload
systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR 2>/dev/null || true
systemctl --user enable --now screenshotd

cat <<'EOF'

Instalação concluída.

Agora adicione ao seu Hyprland:

  hyprland/screenshotd-rules.lua   (configuração em Lua)
  hyprland/screenshotd-keybind.lua (bind do Print)

ou use hyprland/screenshotd.conf se sua configuração for clássica.

Depois:

  hyprctl reload

Teste:

  screenshotd trigger
  systemctl --user status screenshotd
EOF
