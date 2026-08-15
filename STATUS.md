# fastty — Status & Pendientes

**Última actualización:** 2026-08-09
**Plataforma objetivo:** macOS únicamente (Linux/Windows no se tocan)

---

## 1. Optimización de startup ✅

| Cambio | Archivo | Impacto |
|--------|---------|---------|
| Fontconfig (fc-list/fc-match) solo en Linux | `src/renderer/atlas.rs` | Evita ~200ms de subprocess spam en macOS |
| Backends específicos por plataforma (METAL en macOS) | `src/renderer/mod.rs` | Elimina enumeración innecesaria de GPUs |
| Shell integration escrita una vez por proceso (`OnceLock`) | `src/terminal_state.rs` | Elimina reescritura de archivos en cada nueva pestaña |

**Resultado:** Cold start ~129ms → Warm ~79ms (al primer event loop iteration).

---

## 2. Chrome scaling en Retina (2×) ✅ parcial — pendiente bugs

### Problema
En pantallas Retina (scale_factor=2.0), el pipeline dibuja en **píxeles físicos** pero las constantes del chrome estaban en unidades lógicas de diseño. Resultado:
- Topbar: ~15 logical px (debería ~30-40)
- Iconos/settings: mitad de tamaño
- Logo: mitad de tamaño

### Solución aplicada
- `chrome_layout.rs`: Factor global `set_scale(f32)` con `AtomicU32` (milli-units)
- `pipeline.rs`: `sf = chrome_layout::scale()` → todas las constantes del chrome multiplicadas por `sf`
- `main.rs`: Hit-testing alineado con la geometría escalada

### Archivos modificados
- `src/chrome_layout.rs` — `set_scale()`, `scale()`, `scale_f64()`, `topbar_bottom_f64()`
- `src/renderer/pipeline.rs` — topbar, tabs, buttons, logo, scrollbar, rename input
- `src/main.rs` — umbrales de click, context menu, cursor_outside_tab_area

---

## 3. Bugs conocidos / Pendientes

| # | Bug | Severidad | Notas |
|---|-----|-----------|-------|
| 1 | **Ventana "About"**: topbar usa `40.0` hardcoded sin escalar | Baja | Ventana popup separada, no afecta la app principal |
| 2 | **Ventana "Settings"**: topbar tiene su propio `scale = viewport_width/400` | Baja | Independiente del chrome principal, parece funcionar |
| 3 | **Context menu**: `position.x` para el ancho del menú no escala | Baja | Afecta posición horizontal del menú contextual |
| 4 | **Scrollbar track en second window**: `track_top = topbar_h` funciona, pero offsets internos pueden estar descaliados | Media | Revisar si el scrollbar responde correctamente en ventanas secundarias |
| 5 | **Doble verificación de startup**: `cargo fix` dejó warnings de `is_color_font` sin usar | Baja | Preexistente, no relacionado |
| 6 | **Necesita prueba visual**: Todos los cambios de rendering son teóricos — no se verificaron con screenshots | Alta | Abrir la app y confirmar que el topbar, logo, settings e íconos se ven correctos |

---

## 4. Archivos clave (referencia rápida)

| Archivo | Función |
|---------|---------|
| `src/chrome_layout.rs` | Rects de UI del chrome, factor de escala global |
| `src/renderer/pipeline.rs` | Dibujado de todo el chrome (topbar, tabs, logo, settings, scrollbar) |
| `src/renderer/mod.rs` | Selección de adapter/wgpu, llamada a `render()` |
| `src/main.rs` | Hit-testing, drag tabs, context menus, control flow |
| `src/terminal_state.rs` | PTY spawn, shell integration |
| `src/font_discovery_macos.rs` | CoreText font resolution |
| `src/macos_metal_layer.rs` | CAMetalLayer setup |

---

## 5. Próximos pasos (cuando se reanude)

1. **Verificar visualmente** — abrir la app en Retina y confirmar que el topbar mide ~30-40 logical px, logo y settings se ven proporcionales
2. **Ventana About** — aplicar `sf` al topbar de `render_about()` si se desea consistencia
3. **Context menu X** — escalar `context_menu_x = 8.0` si es necesario
4. **Scroll vertical en second window** — revisar offsets del scrollbar
5. **Considerar refactor** — la lógica de tabs en `main.rs` (~2000 líneas) duplica geometría de `pipeline.rs`. Un módulo compartido `tab_layout` eliminaría esta duplicación y reduciría bugs de sincronización

---

## 6. Construcción

```bash
cargo build --release    # ~54s en M1
./target/release/fastty  # smoke test: kill después de 2s
```
