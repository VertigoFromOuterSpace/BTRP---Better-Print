# BTRP — Better Print

Daemon + overlay de captura de tela em **Rust** para **Hyprland/Wayland**.

O `Print` abre uma captura congelada da tela; você seleciona a região com o
mouse, desenha/anota e aperta **Enter** para copiar o PNG para o clipboard.

## O que faz

- Prende a imagem da tela no momento do `Print`.
- A seleção começa exatamente no ponto em que você pressiona o mouse.
- Cursor em círculo branco com borda preta, para precisão.
- Ferramentas de anotação:
  - **P** caneta
  - **L** linha
  - **R** retângulo
  - **E** elipse
  - **A** seta
  - **H** marca-texto
  - **B** pixelar (útil para esconder dados sensíveis)
  - **S** seleção
- **Ctrl+Z** / **Backspace** desfaz a última anotação.
- **Enter** recorta a seleção, aplica as anotações e copia o PNG.
- **Esc** cancela.
- As cores da borda/cursor/accent vêm do tema ativo do Quickshell
  (`~/.local/state/hypr/theme.json`), com fallback para PostalCorpse.

## Requisitos

- Linux com Wayland e **Hyprland**.
- `grim` (captura) e `wl-clipboard` (`wl-copy`).
- Rust recente para compilar (recomendado 1.90+).
- systemd de usuário para manter o daemon.

No Debian/Kali:

```sh
sudo apt install grim wl-clipboard cargo
```

## Instalação

```sh
git clone git@github.com:VertigoFromOuterSpace/BTRP---Better-Print.git
cd BTRP---Better-Print
./install.sh
```

O `install.sh` compila em release, instala o binário em `~/.local/bin/screenshotd`,
instala a unit em `~/.config/systemd/user/screenshotd.service` e habilita/inicia
o serviço. Ele **não altera** sua configuração do Hyprland.

## Configuração do Hyprland

### Configuração em Lua

Adicione as regras ao seu `rules.lua`:

```lua
hl.window_rule({
  match = { class = "^(screenshotd)$" },
  float = true,
  pin = true,
  stay_focused = true,
  size = { "(monitor_w)", "(monitor_h)" },
  move = { "0", "0" },
  no_anim = true,
  no_shadow = true,
})
```

E o bind ao `keybinds.lua`:

```lua
hl.bind("Print", hl.dsp.exec_cmd("screenshotd trigger"), {
  description = "Screenshot (screenshotd)",
})
```

Se `screenshotd` não estiver no `PATH`, use o caminho absoluto:

```lua
hl.dsp.exec_cmd(os.getenv("HOME") .. "/.local/bin/screenshotd trigger")
```

### Configuração clássica (`hyprland.conf`)

```ini
windowrulev2 = float,class:^(screenshotd)$
windowrulev2 = pin,class:^(screenshotd)$
windowrulev2 = stayfocused,class:^(screenshotd)$
windowrulev2 = size 100% 100%,class:^(screenshotd)$
windowrulev2 = move 0 0,class:^(screenshotd)$
windowrulev2 = noanim,class:^(screenshotd)$
windowrulev2 = noshadow,class:^(screenshotd)$

bind = , Print, exec, screenshotd trigger
```

Depois recarregue o Hyprland:

```sh
hyprctl reload
```

## Uso

1. Pressione **Print**.
2. Arraste para selecionar a região.
3. Escolha uma ferramenta e desenhe, se quiser.
4. **Enter** copia o PNG para o clipboard.
5. **Esc** cancela.

O arquivo não é salvo automaticamente; o resultado vai para o clipboard.

## Tema

O daemon lê `~/.local/state/hypr/theme.json` a cada captura:

```json
{
  "colors": {
    "background": "#0a0a0a",
    "foreground": "#f2f2f2",
    "cursor": "#c62828",
    "color1": "#c62828",
    "color5": "#c62828"
  }
}
```

Se o arquivo não existir, usa as cores padrão do PostalCorpse.

## Persistência

O serviço é de usuário:

```sh
systemctl --user status screenshotd
systemctl --user restart screenshotd
journalctl --user -u screenshotd -f
```

Em alguns ambientes, o systemd de usuário não recebe o `WAYLAND_DISPLAY`:

```sh
systemctl --user import-environment WAYLAND_DISPLAY XDG_RUNTIME_DIR
systemctl --user restart screenshotd
```

Se você inicia o Hyprland por um script, pode chamar isso e `screenshotd trigger`
a partir do próprio Hyprland.

## Compilar manualmente

```sh
cargo build --release
install -Dm755 target/release/screenshotd ~/.local/bin/screenshotd
install -Dm644 systemd/screenshotd.service ~/.config/systemd/user/screenshotd.service
systemctl --user daemon-reload
systemctl --user enable --now screenshotd
```

## Solução de problemas

- **O Print não abre nada:** confira `systemctl --user is-active screenshotd` e
  `journalctl --user -u screenshotd -n 50`.
- **Abre, mas o daemon não vê o Wayland:** importe `WAYLAND_DISPLAY` no systemd
  de usuário e reinicie o serviço.
- **A seleção começa deslocada:** confirme que está usando a versão atual; a
  seleção é capturada no `primary_pressed`, não no limiar de arrasto.
- **A borda não tem a cor do tema:** verifique se
  `~/.local/state/hypr/theme.json` existe e contém `colors.cursor` ou
  `colors.color5`.
- **Tiling do Hyprland:** as regras de `float`, `pin` e `stay_focused` mantêm o
  overlay fora do tiling. Sem elas, o Hyprland pode redimensionar a janela.

## Limitações

- O resultado é copiado para o clipboard; não há salvamento automático.
- Ainda não há ferramenta de texto.
- Validado em um monitor 1920x1080; múltiplos monitores podem exigir ajustes de
  geometria.
- Requer Hyprland/Wayland; não é um aplicativo X11.
