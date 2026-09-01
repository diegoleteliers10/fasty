/* tslint:disable */
/* eslint-disable */

export class FasttyVt {
    free(): void;
    [Symbol.dispose](): void;
    clear_dirty(): void;
    cols(): number;
    cursor_col(): number;
    cursor_row(): number;
    cursor_visible(): boolean;
    cwd(): string;
    /**
     * Encode browser KeyboardEvent details into standard ANSI/VT input sequence.
     */
    static encode_key(key: string, ctrl: boolean, alt: boolean, shift: boolean, _meta: boolean): string | undefined;
    feed_bytes(bytes: Uint8Array): void;
    feed_str(text: string): void;
    is_dirty(): boolean;
    max_scroll_offset(): number;
    constructor(cols: number, rows: number, scrollback: number);
    /**
     * High performance direct 2D Canvas renderer. Renders the full terminal grid,
     * background color blocks, styled text, and cursor in batched single passes.
     */
    render_canvas(canvas: HTMLCanvasElement, font_family: string, font_size_px: number, dpr: number): void;
    resize(cols: number, rows: number): void;
    rows(): number;
    /**
     * Scroll display up into history (positive delta) or down towards bottom (negative delta).
     */
    scroll_display(lines: number): void;
    scroll_offset(): number;
    scroll_page_down(): void;
    scroll_page_up(): void;
    scroll_to(offset: number): void;
    scroll_to_bottom(): void;
    scroll_to_top(): void;
    title(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_fasttyvt_free: (a: number, b: number) => void;
    readonly fasttyvt_clear_dirty: (a: number) => void;
    readonly fasttyvt_cols: (a: number) => number;
    readonly fasttyvt_cursor_col: (a: number) => number;
    readonly fasttyvt_cursor_row: (a: number) => number;
    readonly fasttyvt_cursor_visible: (a: number) => number;
    readonly fasttyvt_cwd: (a: number, b: number) => void;
    readonly fasttyvt_encode_key: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly fasttyvt_feed_bytes: (a: number, b: number, c: number) => void;
    readonly fasttyvt_is_dirty: (a: number) => number;
    readonly fasttyvt_max_scroll_offset: (a: number) => number;
    readonly fasttyvt_new: (a: number, b: number, c: number) => number;
    readonly fasttyvt_render_canvas: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly fasttyvt_resize: (a: number, b: number, c: number) => void;
    readonly fasttyvt_rows: (a: number) => number;
    readonly fasttyvt_scroll_display: (a: number, b: number) => void;
    readonly fasttyvt_scroll_offset: (a: number) => number;
    readonly fasttyvt_scroll_page_down: (a: number) => void;
    readonly fasttyvt_scroll_page_up: (a: number) => void;
    readonly fasttyvt_scroll_to: (a: number, b: number) => void;
    readonly fasttyvt_scroll_to_bottom: (a: number) => void;
    readonly fasttyvt_scroll_to_top: (a: number) => void;
    readonly fasttyvt_title: (a: number, b: number) => void;
    readonly fasttyvt_feed_str: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export3: (a: number, b: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number, d: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
