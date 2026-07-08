# Plan: Corrección de renderizado de celdas, UTF-8/emoji y atlas en fasty

## Contexto para el agente

`fasty` es un emulador de terminal en Rust (wgpu + winit + alacritty_terminal + freetype-rs).
El grid de celdas viene de `alacritty_terminal::Term`, se recorre en
`src/renderer/pipeline.rs`, y los glifos se rasterizan/cachean en un atlas de textura
GPU en `src/renderer/atlas.rs` (shelf-packing con FreeType).

Este plan corrige 5 problemas encontrados en una auditoría del código actual, comparando
contra las prácticas de Ghostty y Warp. Están ordenados por prioridad de impacto real.
Cada fase es independiente y debe dejar el proyecto compilando y renderizando antes de
pasar a la siguiente. No mezclar fases en un mismo commit.

**Regla general:** todo cambio de rendering debe probarse visualmente con:
```bash
chafa --symbols block imagen.png      # bloques clásicos (ya funciona)
chafa --symbols braille imagen.png    # Braille — el bug activo
chafa --symbols legacy imagen.png     # sextantes/octantes
echo "👨‍👩‍👧‍👦 🏳️‍🌈 👩‍💻 ✏️ ✂️"       # ZWJ + VS16
printf 'e\u0301\n'                     # combining accent (é compuesto)
```

---

## Fase 1 — Dibujo procedural de Braille y Legacy Computing (prioridad máxima, es el bug activo)

**Objetivo:** eliminar el drift horizontal en ASCII art de chafa dibujando Braille y
sextantes/octantes vectorialmente en vez de depender del glyph advance de la fuente
instalada, igual que ya se hace con `decode_box_drawing`.

### 1.1 Extender la detección de "glifo procedural"

Archivo: `src/renderer/pipeline.rs`

Renombrar/extender `is_custom_block_drawing` para cubrir los rangos nuevos (o crear
funciones hermanas — decisión de estilo del agente, pero deben usarse consistentemente
en todos los call sites que hoy llaman a `is_custom_block_drawing` e `is_block_element`):

```rust
pub fn is_custom_block_drawing(ch: char) -> bool {
    matches!(ch as u32,
        0x2500..=0x259F |   // box drawing + block elements (ya existente)
        0x2800..=0x28FF |   // Braille Patterns
        0x1FB00..=0x1FBFF   // Symbols for Legacy Computing (sextants, octants, wedges)
    )
}
```

Nota: revisar si `0xE0B0..=0xE0B3` (Powerline) conviene incluirlo aquí también — ver
Fase 1.4 antes de decidir, porque Powerline normalmente sí se apoya en Nerd Fonts para
mantener consistencia visual con el resto del prompt, y dibujarlo procedural puede
verse distinto al resto de los glifos powerline no cubiertos (hay más símbolos powerline
que solo esos 4 codepoints). Evaluar con capturas antes/después.

### 1.2 Decodificador de Braille

Cada codepoint Braille `U+2800–U+28FF` codifica 8 puntos en una máscara de bits sobre
una grilla de 2 columnas × 4 filas (bit 0 = arriba-izquierda, ... ver mapeo estándar
Unicode de Braille dot numbering: bits 0-2-4-6 columna izquierda de arriba a abajo salvo
el bit 6 que es el punto 7, y bits 1-3-5-7 columna derecha):

```rust
/// Devuelve las posiciones (col 0-1, fila 0-3) de los puntos activos para un carácter Braille.
pub fn decode_braille(ch: char) -> Option<[bool; 8]> {
    let code = ch as u32;
    if !(0x2800..=0x28FF).contains(&code) {
        return None;
    }
    let mask = (code - 0x2800) as u8;
    // Orden estándar de bits Braille Unicode: 1,2,3,7 (col izq, top->bottom)
    // y 4,5,6,8 (col der, top->bottom)
    Some([
        mask & 0b0000_0001 != 0, // punto 1 -> (0,0)
        mask & 0b0000_0010 != 0, // punto 2 -> (0,1)
        mask & 0b0000_0100 != 0, // punto 3 -> (0,2)
        mask & 0b0100_0000 != 0, // punto 7 -> (0,3)
        mask & 0b0000_1000 != 0, // punto 4 -> (1,0)
        mask & 0b0001_0000 != 0, // punto 5 -> (1,1)
        mask & 0b0010_0000 != 0, // punto 6 -> (1,2)
        mask & 0b1000_0000 != 0, // punto 8 -> (1,3)
    ])
}
```

En el punto de dibujo (ver 1.3), cada punto activo se pinta como un círculo (o cuadrado
redondeado, más barato de rasterizar) centrado en `(col * cell_w/2, row * cell_h/4)`,
con radio proporcional a `min(cell_w, cell_h) * 0.12` aprox — calibrar visualmente
contra una fuente que sí tenga Braille bien alineado (ej. DejaVu Sans Mono) para que el
tamaño de punto se vea nativo.

### 1.3 Decodificador de sextantes/octantes (Legacy Computing)

Los sextantes (`U+1FB00–U+1FB3B`) dividen la celda en una grilla de 2×3 (6 sub-bloques).
Los octantes (más nuevos, extensión posterior del mismo bloque) usan 2×4. El mapeo
bit→posición sigue el orden de la tabla Unicode "Symbols for Legacy Computing" (bloque
`Sextant-1` a `Sextant-63`, cada codepoint = una combinación de los 6 bits de sub-bloque
activos, en orden: top-left, top-right, mid-left, mid-right, bottom-left, bottom-right).

```rust
/// Devuelve qué sub-bloques (2 cols x 3 filas) están rellenos para un sextante dado.
pub fn decode_sextant(ch: char) -> Option<[bool; 6]> {
    let code = ch as u32;
    // Rango base de sextantes (verificar offset exacto contra la tabla Unicode 15.0
    // "Symbols for Legacy Computing", sección Sextants, antes de mergear).
    if !(0x1FB00..=0x1FB3B).contains(&code) {
        return None;
    }
    let idx = (code - 0x1FB00) as u32;
    // idx codifica los 6 bits de sub-bloque en orden fila-mayor (TL,TR,ML,MR,BL,BR).
    // OJO: en la tabla real de Unicode los codepoints NO son secuenciales 0..63 en ese
    // orden exacto (hay saltos porque los 2 casos "todo lleno"/"todo vacío" ya existen
    // como block elements 0x2588/espacio y se excluyen del bloque sextants). El agente
    // debe generar esta tabla desde el archivo oficial de Unicode (ver Fase 1.5), NO
    // adivinar el mapeo bit a bit a mano — es propenso a errores sutiles de 1 bit que
    // se notan como sub-bloques en la posición equivocada.
    todo!("generar desde datos Unicode oficiales, ver sección 1.5")
}
```

**Importante:** a diferencia de Braille (que sí es un mapeo lineal directo de bits), el
bloque de sextantes tiene un orden de codepoints que NO es una simple numeración binaria
secuencial. No inventar el mapeo — usar la tabla de referencia real (ver 1.5).

### 1.4 Punto de dibujo en el pipeline

En `src/renderer/pipeline.rs`, buscar el bloque que ya maneja
`is_custom_block_drawing(cell.c)` y `decode_box_drawing` (cerca de línea ~5862 y su
call site en el loop de foreground). Agregar ramas equivalentes:

```rust
if let Some(dots) = decode_braille(cell.c) {
    draw_braille_cell(&mut bg_instances /* o buffer que corresponda */, cell_x, cell_y,
                       actual_cell_width, actual_cell_height, dots, fg_color);
    continue; // no pasar por el path de rasterización de fuente para este char
}
if let Some(sextant) = decode_sextant(cell.c) {
    draw_sextant_cell(..., cell_x, cell_y, actual_cell_width, actual_cell_height,
                       sextant, fg_color);
    continue;
}
```

`draw_braille_cell` y `draw_sextant_cell` deben generar instancias geométricas (mismo
mecanismo que ya usa `decode_box_drawing` para emitir rectángulos — reusar esa
infraestructura de instancing, no crear un pipeline gráfico nuevo).

### 1.5 Generar la tabla de sextantes desde datos oficiales, no a mano

Descargar `https://www.unicode.org/Public/UCD/latest/ucd/UnicodeData.txt` o el archivo
específico de bloques Legacy Computing, y generar la tabla de mapeo bit↔codepoint en
build time (`build.rs`, que el proyecto ya usa para otra cosa) o como tabla estática
generada una vez y commiteada (`const SEXTANT_TABLE: [[bool;6]; N]`). Esto evita el
riesgo de un mapeo manual con errores de 1 bit que después son muy difíciles de detectar
a simple vista.

### Criterio de aceptación Fase 1

- `chafa --symbols braille` sobre una imagen ancha ya no muestra corrimiento horizontal
  progresivo de fila a fila ni de columna a columna.
- `chafa --symbols legacy` (sextantes) se ve alineado igual de bien.
- Comparar pixel a pixel (captura de pantalla) contra Ghostty renderizando la misma
  imagen con el mismo ancho de terminal — deben coincidir en alineación aunque no en
  estilo exacto de punto/densidad.

---

## Fase 2 — Padding en el atlas (bajo esfuerzo, resuelve bleeding)

Archivo: `src/renderer/atlas.rs`, struct `ShelfPacker`.

```rust
impl ShelfPacker {
    const GLYPH_PADDING: u32 = 2; // px de margen transparente alrededor de cada glifo

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let padded_w = w + Self::GLYPH_PADDING;
        let padded_h = h + Self::GLYPH_PADDING;

        if self.cursor_x + padded_w > self.width {
            self.cursor_x = 0;
            self.cursor_y += self.row_height;
            self.row_height = 0;
        }
        if self.cursor_y + padded_h > self.height {
            return None;
        }

        // El glifo real se ubica con medio padding de margen en cada borde.
        let x = self.cursor_x + Self::GLYPH_PADDING / 2;
        let y = self.cursor_y + Self::GLYPH_PADDING / 2;

        self.cursor_x += padded_w;
        self.row_height = self.row_height.max(padded_h);

        Some((x, y))
    }
}
```

Verificar que todos los call sites de `alloc` (hay varios en `atlas.rs`, uno por cada
variante de carga: freetype normal, freetype color, SVG rasterizado, etc.) sigan
funcionando igual — el contrato de retorno `(x, y)` no cambia, solo el uso interno de
espacio en la textura.

**No tocar** el inset de 0.5px que ya existe en `AtlasEntry::uv_coords` para `is_block`
— sigue siendo necesario, es un fix distinto y complementario (uno es margen físico en
la textura, el otro es offset de sampling UV).

### Criterio de aceptación Fase 2

- Con zoom/DPI fraccional (ej. 125%, 150% de escala en el sistema), no debe verse
  ninguna franja de color de un glifo "sangrando" sobre el glifo vecino en el atlas.
- Revisar visualmente el atlas completo (si hay algún modo debug para volcarlo a PNG,
  usarlo; si no, agregar temporalmente uno) para confirmar que hay margen visible entre
  glifos empaquetados.

---

## Fase 3 — Limpiar/aprovechar la dependencia `unicode-width`

Archivo: `Cargo.toml`.

`unicode-width = "0.1"` está declarada pero no se usa en ningún archivo de `src/`
(confirmado con `grep -rn "unicode_width\|UnicodeWidthChar\|UnicodeWidthStr" src/`).

Dos caminos, elegir uno:

- **(a) Eliminar la dependencia** si la Fase 4 (zerowidth/ZWJ) no se va a implementar en
  el corto plazo. Simplemente borrar la línea de `Cargo.toml` y correr
  `cargo build` para confirmar que nada se rompe.
- **(b) Dejarla y empezar a usarla activamente** en la Fase 4, para el cálculo de ancho
  visual de clusters de grafemas completos (no solo un `char`). Si se elige esta opción,
  subir la versión a la más reciente (`unicode-width = "0.2"` o superior al momento de
  implementar) — la 0.1 tiene datos Unicode más viejos.

Si se elige (b), no cerrar esta fase hasta después de completar la Fase 4, ya que ahí se
decide el uso real.

---

## Fase 4 — Soporte de ZWJ, selectores de variación y marcas combinantes (mayor esfuerzo)

**Objetivo:** dejar de perder secuencias emoji compuestas (👨‍👩‍👧‍👦, 🏳️‍🌈, 👩‍💻),
selectores de presentación emoji (`U+FE0F`) y marcas combinantes (acentos sueltos).

### 4.1 Contexto técnico

`alacritty_terminal::term::cell::Cell` expone:
```rust
pub fn zerowidth(&self) -> Option<&[char]>
```
que devuelve los caracteres de ancho cero (combining marks, continuaciones ZWJ,
selectores de variación) adjuntos a la celda base. Confirmado con
`grep -rn "zerowidth" src/` → **cero resultados** en todo el proyecto. Este método
nunca se llama, ni en el path de shaping con ligaduras ni en el path cell-por-cell.

### 4.2 Dónde tocar

Archivo: `src/renderer/pipeline.rs`, en la construcción de `row_text` para el shaping
con `rustybuzz` (cerca de línea ~1090-1120):

```rust
// ANTES — solo toma el char base, descarta zerowidth:
for cell in cells.iter() {
    row_text.push(cell.c);
}
```

```rust
// DESPUÉS — anexa los zerowidth después del char base, y ajustar col_map en consecuencia:
for cell in cells.iter() {
    row_text.push(cell.c);
    if let Some(extra) = cell.zerowidth() {
        for &zw in extra {
            row_text.push(zw);
        }
    }
}
```

El `col_map` (que mapea cada byte UTF-8 del `row_text` a un índice de columna del grid)
debe extenderse igual, repitiendo el mismo índice de columna para los bytes de los
caracteres zerowidth, ya que visualmente siguen perteneciendo a la misma celda base:

```rust
for (idx, cell) in cells.iter().enumerate() {
    let mut char_len = cell.c.len_utf8();
    if let Some(extra) = cell.zerowidth() {
        char_len += extra.iter().map(|c| c.len_utf8()).sum::<usize>();
    }
    for _ in 0..char_len {
        col_map.push(idx);
    }
}
```

Esto permite que HarfBuzz/rustybuzz vea la secuencia completa (base + ZWJ + siguiente
emoji + VS16, etc.) y aplique correctamente las reglas de shaping/ligadura de la fuente
de emoji, produciendo el glifo compuesto en vez de solo el primero.

### 4.3 Ajustar la detección de "es emoji/color" para considerar VS16

Con el VS16 (`U+FE0F`) ahora presente en `row_text`, la lógica de `is_emoji(cell.c)` en
`src/renderer/atlas.rs` sigue mirando solo el char base y no sabrá que viene seguido de
VS16. Agregar una variante que reciba el contexto zerowidth:

```rust
pub fn is_emoji_presentation(ch: char, zerowidth: Option<&[char]>) -> bool {
    if is_emoji(ch) {
        return true;
    }
    // Si el char base viene con selector de presentación emoji (VS16),
    // forzar presentación a color aunque el codepoint base no esté en la tabla
    // (cubre casos como ✏️ ✂️ ⚡ del bloque Dingbats, hoy excluido a propósito).
    matches!(zerowidth, Some(zw) if zw.contains(&'\u{FE0F}'))
}
```

Actualizar el call site en el loop de foreground (donde hoy se calcula
`is_emoji_or_block_or_wide`) para pasar `cell.zerowidth()`.

### 4.4 Caso especial: marcas combinantes puras (sin emoji)

Para el caso de acentos combinantes sueltos (ej. `e` + `U+0301`), el comportamiento
correcto NO es tratarlos como color/emoji — deben dibujarse superpuestos sobre el
glifo base usando el glifo normal de la marca combinante en la fuente de texto, en la
posición vertical que le corresponda (encima, según la métrica de la fuente). Con el
cambio de 4.2 ya llegan a `rustybuzz`, que —si la fuente tiene soporte de shaping de
marcas (GPOS `mark`/`mkmk`)— debería posicionarlos correctamente solo. Verificar con la
fuente monoespaciada que uses por defecto (JetBrains Mono, etc.) si tiene esas tablas;
si no las tiene, puede requerir un fallback manual de posicionamiento (fuera de alcance
de este plan si no aparece en las pruebas — dejar como nota para una fase futura si se
detecta el problema).

### Criterio de aceptación Fase 4

- `echo "👨‍👩‍👧‍👦"` renderiza la familia completa como un solo glifo compuesto (o al
  menos visualmente unida), no solo 👨.
- `echo "🏳️‍🌈"` renderiza la bandera arcoíris, no solo 🏳.
- `echo "✏️ ✂️ ⚡"` renderiza a color (antes se perdían por estar en Dingbats excluido).
- `printf 'e\u0301'` muestra é (o al menos e con acento visible), no "e" pelado.
- No debe haber regresión: texto normal sin ZWJ/combining sigue viéndose exactamente
  igual que antes (correr el set de pruebas visual existente del proyecto si hay uno).

---

## Fase 5 — Reemplazar la tabla de emoji hardcodeada por datos Unicode oficiales

**Objetivo:** dejar de mantener a mano `is_emoji()` en `src/renderer/atlas.rs:1598`, que
tiene al menos un error confirmado (rango de banderas empieza en `0x1F1E0` en vez de
`0x1F1E6`, el valor real de inicio de Regional Indicator Symbols) y excluye por completo
el bloque Dingbats salvo excepciones puntuales.

### 5.1 Generar la tabla en build time

En `build.rs` (ya existe en el proyecto para otra cosa), agregar un paso que:
1. Descargue o incluya como recurso local
   `https://unicode.org/Public/emoji/latest/emoji-data.txt` (propiedades `Emoji`,
   `Emoji_Presentation`, `Extended_Pictographic`).
2. Parsee los rangos y genere un archivo Rust (`OUT_DIR/emoji_table.rs`) con un `match`
   o tabla de rangos ordenada, similar en forma a la actual pero exhaustiva y correcta.
3. Incluir ese archivo generado vía `include!(concat!(env!("OUT_DIR"), "/emoji_table.rs"))`
   en `atlas.rs`, reemplazando el cuerpo actual de `is_emoji`.

Si no se quiere depender de red en build time (problemas de reproducibilidad/CI),
alternativa: vendorizar `emoji-data.txt` como archivo commiteado en el repo (ej.
`data/emoji-data.txt`) y generar la tabla desde ahí en cada build. Preferir esta opción
para no depender de conectividad al compilar.

### 5.2 Mantener el fallback runtime existente

No eliminar la corrección por `pixel_mode == PixelMode::Bgra` en `atlas.rs:778`
(`actual_is_color = is_color || pixel_mode == freetype::bitmap::PixelMode::Bgra ||
is_emoji_char`). Esa señal runtime sigue siendo la fuente de verdad final y debe
mantenerse como red de seguridad incluso con la tabla generada correctamente — cubre
fuentes con glifos de color en codepoints no anticipados por la tabla.

### Criterio de aceptación Fase 5

- La bandera 🇨🇱 (Chile, `U+1F1E8 U+1F1F1`) sigue detectándose como emoji (regression
  check, ya que el rango correcto `0x1F1E6..=0x1F1FF` sigue cubriéndola).
- Emojis Dingbat con VS16 ya cubiertos por la Fase 4 no dependen ahora de una excepción
  manual sino de la combinación tabla-oficial + detección VS16.
- `cargo build` sin conexión a internet sigue funcionando (si se vendorizó el archivo
  fuente de datos).

---

## Orden de ejecución recomendado

1. **Fase 1** (Braille/sextantes) — resuelve el bug activo, es la de mayor impacto visible.
2. **Fase 2** (padding atlas) — bajo esfuerzo, sin dependencias de las otras fases.
3. **Fase 3** (a o b, decidir según si se va a hacer la Fase 4 pronto).
4. **Fase 4** (ZWJ/VS16/combining) — la de mayor esfuerzo, pero la de mayor paridad con
   Ghostty/Warp en soporte real de emoji moderno.
5. **Fase 5** (tabla emoji desde datos oficiales) — depende de haber decidido cómo se
   usa VS16 en la Fase 4, para que la tabla y la detección de VS16 trabajen juntas sin
   duplicar lógica.

Cada fase debe cerrarse con: `cargo build --release`, pasar el set de pruebas visuales
de la sección "Contexto para el agente", y un commit separado por fase (no un solo PR
gigante) para poder hacer bisect si algo regresiona visualmente.
