-- Overlay do screenshotd: janela flutuante em tela cheia, fora do tiling.
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
