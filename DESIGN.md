# Fastty Design System

This document specifies the design system for Fastty.
Fastty is a fast terminal emulator built on GPUI.
The design system provides a clear visual hierarchy, deterministic color tokens, and modular components.

---

## 1. Design Principles

1. **Clarity First**: The user must read text without eye strain. Interface chrome remains subtle and gives priority to terminal content.
2. **Speed and Determinism**: Every interaction gives immediate feedback. State transitions do not block user input.
3. **Structured Grouping**: Settings and modal dialogs use inset grouped cards with clear visual borders. This matches the macOS and Raycast visual style.
4. **Token Consistency**: Components read all colors and spacing from the active theme. Do not use hard-coded color values in view layouts.

---

## 2. Color Palette and Token System

The file `src/ui/theme.rs` defines the theme system.
All colors use the GPUI `Hsla` format.

### 2.1 Core Chrome Tokens

These tokens style application chrome, sidebars, cards, and input fields:

| Token Name | Hex Default | Usage |
| :--- | :--- | :--- |
| `foreground` | `#E5E9F0` | Primary text and default icon tint |
| `background` | `#151515` | Root window background and terminal canvas |
| `main_bg` | `#151515` | Main viewport fill |
| `sidebar_bg` | `#151515` | Background for left and right navigation panels |
| `tab_bar_bg` | `#151515` | Tab bar background bar |
| `tab_active_bg` | `#202020` | Active tab surface background |
| `tab_inactive_bg` | `#101010` | Inactive tab surface background |
| `status_bar_bg` | `#151515` | Status bar fill |
| `surface` | `#151515` | Base background for cards and input containers |
| `surface_raised` | `#202020` | Elevated cards, dialog boxes, and tool containers |
| `border` | `#2A2A2A` | 1px border stroke for cards, dividers, and panels |
| `hover` | `#262626` | Interactive hover background for rows and buttons |
| `selected` | `#303030` | Active selection fill for list items and text blocks |
| `muted` | `#848B98` | Secondary labels, hints, and disabled controls |
| `muted_strong` | `#A0A8B6` | Readable secondary text and breadcrumbs |
| `accent` | `#FDA906` | Brand amber color for active focus, cursors, and badges |
| `cursor` | `#FDA906` | Terminal cursor and input field blinking bar |

### 2.2 ANSI Terminal Palette

These tokens control terminal emulation colors and status indicators:

| ANSI Slot | Standard Hex | Bright Hex | Semantic Intent |
| :--- | :--- | :--- | :--- |
| Black | `#3B4252` | `#848B98` | Muted elements and dark borders |
| Red | `#F04E4E` | `#FF6B6B` | Errors, dangerous actions, and test failures |
| Green | `#8EE044` | `#A6F05E` | Success state, active git branch, installed tags |
| Yellow | `#FDA906` | `#FFE04A` | Warnings, pending approvals, highlight items |
| Blue | `#5AB0F0` | `#74CCFF` | Information chips, links, process IDs |
| Magenta | `#D07EE0` | `#E896FA` | AI reasoning deltas and special highlights |
| Cyan | `#56E2DB` | `#6AF5F0` | Paths, tool arguments, and metadata |
| White | `#E5E9F0` | `#FFFFFF` | Bright text and emphasized labels |

### 2.3 Window Opacity and Transparency

The theme supports variable window opacity through `Theme::with_opacity`:

- When `opacity < 1.0`, `window_fill()` returns transparent black (`transparent_black()`).
- Chrome layers scale their alpha channel by the configured opacity value.
- Borders and raised surfaces maintain relative contrast above desktop backgrounds.

---

## 3. Built-In Themes

Fastty includes built-in themes and loads custom themes from `~/.config/fastty/themes/*.toml`:

1. **Fastty Default**: High-clarity dark background (`#151515`) with amber accent (`#FDA906`).
2. **Catppuccin Mocha**: Soft pastel dark palette (`#24273A`) with lavender and blue accents.
3. **One Dark**: Atom-inspired balanced dark theme (`#282C34`) with cyan and green highlights.
4. **Solarized Dark**: Precision low-contrast palette (`#002B36`) with teal and yellow accents.
5. **High Contrast**: Pure black background (`#000000`) with pure white foreground (`#FFFFFF`) for maximum accessibility.

---

## 4. Typography and Geometry

### 4.1 Font Roles

Fastty uses two font systems:

1. **Terminal Grid & Code**: Monospace font family. Fastty searches system fonts for JetBrains Mono, Fira Code, Menlo, SF Mono, Hack, and Cascadia Code.
2. **UI Chrome**: Clean proportional or monospace system font. Text remains sharp at all DPI scaling factors.

### 4.2 Type Scale

| Size Token | Value | Applied To |
| :--- | :--- | :--- |
| `micro` | `10px - 11px` | Status bar items, shortcut keycaps, process badges |
| `caption` | `12px` | Input text, card subtitles, tool parameters |
| `body` | `13px` | Card row titles, button labels, list items |
| `heading` | `14px - 15px` | Section titles, sidebar header labels, modal titles |
| `title` | `18px - 20px` | App branding and hero dialog headings |

### 4.3 Corner Radii

| Radius | Value | Usage |
| :--- | :--- | :--- |
| `none` | `0px` | Terminal grid tiles and full-bleed panel dividers |
| `sharp` | `2px` | Cursor bar, text selection spans, active indicators |
| `chip` | `4px` | Model tags, keycaps, small status badges |
| `control` | `6px` | Buttons, text input boxes, dropdown controls |
| `card` | `10px` | Inset grouped cards in settings and drawers |
| `window` | `12px` | Floating windows and modal sheets |

---

## 5. UI Components

Fastty implements modular GPUI components across the application.

### 5.1 Inset Grouped Cards (Settings View)

The settings window (`src/ui/settings_view.rs`) uses Raycast-style inset grouped cards:

- **Card Container**: `rounded(px(10.))`, `border_1()`, `border_color(theme.border)`, `bg(theme.surface)`.
- **Card Rows**: Divided by internal `border_b_1()` borders with color `theme.border`.
- **Row Layout**:
  - Left column: Bold title (`13px`, `theme.foreground`) and muted subtitle (`12px`, `theme.muted`).
  - Right column: Interactive control with `flex_shrink_0()`.
  - The row enforces `min_w(px(0.))` on text to prevent control overflow.

### 5.2 Buttons (`Button`, `ButtonVariant`)

The button component (`src/ui/button.rs`) supports multiple semantic variants:

- **Variants**:
  - `Blurry`: Translucent frosted glass effect using subtle white alpha fills and borders.
  - `Default`: Uses `theme.surface` with `theme.border` and `theme.accent` hover stroke.
  - `Ghost`: Transparent background, subtle white fill on hover.
  - `Dark`: Dark filled background for high-contrast actions.
  - `Light`: Light surface for dark text contrast.
  - `Danger`: Subtle red background with bright red text and border on hover.
  - `Warning`: Amber background for destructive or risky prompts.
  - `Info`: Soft blue background for informational actions.
- **Dimensions**: `px(12.)`, `py(6.)`, `rounded(px(6.))`, `text_size(px(13.))`.

### 5.3 macOS Toggle Switch

The toggle switch provides standard binary settings control:

- **Track**: Pill shape (`w(px(28.))`, `h(px(16.))`, `rounded(px(8.))`).
- **Active State**: Track background switches to `theme.accent`. White round knob shifts to the right edge.
- **Inactive State**: Track background uses `theme.surface_raised`. Muted round knob sits on the left edge.

### 5.4 Text Input System (`TextInputState`)

Text input components (`src/ui/text_input.rs`) manage text editing without web dependencies:

- **State Management**: Tracks text string, char offset cursor, and drag selection range `(start, end)`.
- **Visual Text Wrapping**: `wrap_text_into_lines` wraps long text into rows by available column width.
- **Rendering**: `render_line_spans` splits lines into text segments, selection highlights (`theme.selected`), and a 2px vertical accent cursor (`theme.accent`).
- **Mouse Interaction**: Single click positions the cursor. Click and drag updates selection ranges.
- **Keyboard Navigation**: Supports arrow navigation, Shift selection, Backspace, Delete forward, Cut, Copy, and Paste.

### 5.5 Tab Bar (`TabBar`)

The tab bar (`src/ui/tab_bar.rs`) supports horizontal top bars and vertical sidebars:

- **Height**: 32px horizontal bar (34px for sidebar header).
- **Active Tab**: `bg(theme.tab_active_bg)`, subtle bottom border, bold title, process icon.
- **Inactive Tab**: `bg(theme.tab_inactive_bg)`, muted label, hover highlight.
- **Controls**: Close button (`✕`) on active tab hover, new tab button (`+`), and AI assistant toggle button (`✦`).

### 5.6 Status Bar (`StatusBar`)

The bottom status bar (`src/ui/status_bar.rs`) reports workspace context:

- **Height**: 20px compact strip.
- **Left Region**: Active working directory, git branch with green dot indicator.
- **Right Region**: Active process name, pane split count, zoom indicator, battery status, current time.

### 5.7 AI Assistant Drawer (`AiSidebar`)

The AI assistant drawer (`src/ui/ai_sidebar.rs`) provides tool execution and LLM chat:

- **Docking**: Right side drawer with dynamic width (clamped between 240px and 750px).
- **Resize Handle**: Single 1px left border (`theme.border`) with an invisible 7px click hitbox (`CursorStyle::ResizeColumn`).
- **Two-Row Header**:
  - Row 1: App icon tile (`[F]`), workspace title (`Fastty AI`), active directory (`~/dev/fasty`), git branch with branch icon (`⑂ main`), new chat button (`+`), history button (`◷`), and close button (`✕`).
  - Row 2: Interactive model selection pill (`{provider}/{model} ⌵`) and context window usage progress bar (`62% contexto`).
- **Message Feed**:
  - User bubble: Speech bubble aligned to the right, styled with `theme.surface_raised` and `theme.border`.
  - Assistant message: Avatar badge (`[F]`), sender name (`Fastty`), and message timestamp (`12:05 PM`).
  - Thinking accordion: Collapsible summary bar with sparkle icon (`✧`) and chevron (`⌵`). Clicking expands the model reasoning block.
  - Tool execution card (`HERRAMIENTAS`): Inset grouped card displaying execution steps (`Ejecutar`, `Buscar`, `Leer`, `Editar`), target paths or commands in monospace, running spinners, and completion status tags.
  - Diff proposal card: File header with `+add` and `-del` indicators, split deletion lines in soft red (`rgba(0xf04e4e1a)`), addition lines in soft green (`rgba(0x8ee0441a)`), and action buttons (`✓ Aplicar cambio`, `✕ Descartar`, `⌘⏎ para aplicar`).
  - Prose formatting: Inline code spans rendered as amber-tinted badges (`theme.surface_raised`, `theme.accent`). Blinking cursor bar during token streaming.
- **Inset Composer Card**:
  - Border highlight: Focus ring switches to `theme.accent`.
  - Input area: Auto-wrapping multi-line text input with drag selection and I-Beam cursor.
  - Toolbar: Segmented mode selector (`[ Agente | Preguntar ]`), mention button (`@`), attachment button, and action button (`Enviar ↑` or `■ Detener`).
  - Keyboard hints: `⌘⏎ enviar · ⇧⏎ nueva línea · esc cerrar`.

### 5.8 Settings Window (`SettingsView`)

The preferences window (`src/ui/settings_view.rs`) uses a two-pane layout:

- **Window Size**: 880px width by 640px height.
- **Left Navigation**:
  - Window drag area with traffic lights clearance.
  - Live search input box.
  - Navigation tabs: `General`, `Appearance`, `Keyboard`, `AI Assistant`, `Migration`, `Advanced`.
- **Right Content Area**:
  - Breadcrumb title with Lucide icon.
  - Escape key hint label.
  - Inset grouped cards for categorized options.
  - BYOK (Bring Your Own Key) model selector with local Ollama auto-detection chips.
  - Manual "Test Connection" button with real-time latency badges.

---

## 6. Icons and Assets

Fastty uses vector SVG icons via `src/ui/icons.rs`:

- **Lucide Icon Registry**: Embedded static SVG paths (`get_icon_svg_bytes`).
- **Application Icon**: `render_app_logo` decodes and renders `assets/fasttySmallIcon.png`.
- **Process Icons**: `get_deck_process_icon` maps common CLI binaries (nvim, git, docker, cargo, python, node) to specific vector icons.
- **Progress Spinners**: `render_spinner` renders an 8-frame smooth vector spinner animation.
