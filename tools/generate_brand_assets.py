#!/usr/bin/env python3
"""Generate src/ui/brand_assets.rs from simple-icons.

Run manually when the brand set changes, then format the result:

    python3 tools/generate_brand_assets.py
    rustfmt --edition 2021 src/ui/brand_assets.rs

Every entry is a full SVG document. Monochrome marks use currentColor so the
theme tints them. Colored marks keep their own fills. The color field is the
brand color, or None when the artwork carries its own colors.
"""

import re
import urllib.request

SIMPLE_ICONS = "https://cdn.simpleicons.org/{}"

SLUGS = {
    "CLAUDE": "claude",
    "OPENCODE": "opencode",
    "GEMINI": "googlegemini",
    "COPILOT": "githubcopilot",
    "CURSOR": "cursor",
    "AMP": "amp",
    "QWEN": "qwen",
    "CLINE": "cline",
    "WINDSURF": "windsurf",
    "MISTRAL": "mistralai",
    "KIMI": "kimi",
    "LM_STUDIO": "lmstudio",
    "NODE": "nodedotjs",
    "BUN": "bun",
    "DENO": "deno",
    "NPM": "npm",
    "PNPM": "pnpm",
    "YARN": "yarn",
    "PYTHON": "python",
    "RUST": "rust",
    "GO": "go",
    "RUBY": "ruby",
    "PHP": "php",
    "JAVA": "openjdk",
    "KOTLIN": "kotlin",
    "SWIFT": "swift",
    "DOTNET": "dotnet",
    "LUA": "lua",
    "ELIXIR": "elixir",
    "ZIG": "zig",
    "DOCKER": "docker",
    "TERRAFORM": "terraform",
    "KUBERNETES": "kubernetes",
    "GIT": "git",
    "NEOVIM": "neovim",
    "VIM": "vim",
    "TMUX": "tmux",
    "REDIS": "redis",
    "SQLITE": "sqlite",
    "POSTGRESQL": "postgresql",
    "MONGODB": "mongodb",
    "CMAKE": "cmake",
    "GRADLE": "gradle",
    "OLLAMA": "ollama",
    "HUGGING_FACE": "huggingface",
    "HASKELL": "haskell",
    "CLOJURE": "clojure",
    "R_LANG": "r",
    "DART": "dart",
    "FLUTTER": "flutter",
}

# Artwork that simple-icons does not carry. The Pi and Antigravity marks come
# from each vendor's own site. Hermes uses an image asset instead.
CUSTOM = {
    "PI": (
        '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 800 800">'
        '<path fill="#F09082" d="M165.29 165.29H517.36V400H400V282.65H165.29Z"/>'
        '<path fill="#4D9ABF" d="M165.29 282.65H282.65V400H400V517.36H282.65V634.72H165.29Z"/>'
        '<path fill="#F1BE58" d="M517.36 400H634.72V634.72H517.36Z"/>'
        "</svg>",
        None,
    ),
    "ANTIGRAVITY": (
        '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 136 136">'
        '<g transform="translate(12 10)">'
        '<path fill="#3186FF" d="M89.6992 93.695C94.3659 97.195 101.366 94.8617 94.9492 88.445C75.6992 69.7783 79.7825 18.445 55.8659 18.445C31.9492 18.445 36.0325 69.7783 16.7825 88.445C9.78251 95.445 17.3658 97.195 22.0325 93.695C40.1159 81.445 38.9492 59.8617 55.8659 59.8617C72.7825 59.8617 71.6159 81.445 89.6992 93.695Z"/>'
        "</g></svg>",
        0x3186FF,
    ),
}

OPENAI_PATHS = (
    "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 "
    "6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 "
    "7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 "
    "13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 "
    "6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 "
    "4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826"
    "a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 "
    "4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 "
    "19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6"
    "a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865"
    "A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 "
    "0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231"
    "l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 "
    ".0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804"
    ".0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948"
    ".7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997"
    "-2.6067-1.4997Z"
)


def fetch(slug):
    request = urllib.request.Request(
        SIMPLE_ICONS.format(slug), headers={"User-Agent": "fastty-brand-assets"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        text = response.read().decode("utf-8")
    fill = re.search(r'fill="(#[0-9A-Fa-f]{6})"', text)
    paths = re.findall(r"<path[^>]*d=\"([^\"]+)\"", text)
    if not paths:
        raise SystemExit(f"{slug}: no path data")
    body = "".join(f'<path d="{d}"/>' for d in paths)
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" '
        f'viewBox="0 0 24 24" fill="currentColor">{body}</svg>'
    )
    color = int(fill.group(1)[1:], 16) if fill else None
    return svg, color


def const(name, svg, color):
    color_expr = "None" if color is None else f"Some(0x{color:06X})"
    return (
        f'pub const {name}: BrandArtwork = BrandArtwork {{ svg: r##"{svg}"##, color: {color_expr} }};\n' 
    )


def main():
    entries = []
    for name, slug in SLUGS.items():
        try:
            svg, color = fetch(slug)
        except Exception as error:  # noqa: BLE001 - report and keep going
            print(f"skipped {name} ({slug}): {error}")
            continue
        entries.append(const(name, svg, color))
    for name, (svg, color) in CUSTOM.items():
        entries.append(const(name, svg, color))
    openai_svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" '
        f'viewBox="0 0 24 24" fill="currentColor"><path d="{OPENAI_PATHS}"/></svg>'
    )
    entries.append(const("OPENAI", openai_svg, None))

    header = '''//! Official brand artwork for tab and notification marks.
//!
//! Generated by `tools/generate_brand_assets.py`. Do not edit by hand.
//! Each entry is a full SVG document. Monochrome marks use `currentColor`, so
//! the theme tints them. `color` is the brand color, or `None` when the
//! artwork carries its own colors.

pub struct BrandArtwork {
    pub svg: &'static str,
    pub color: Option<u32>,
}

'''
    with open("src/ui/brand_assets.rs", "w", encoding="utf-8") as handle:
        handle.write(header)
        handle.write("".join(entries))
    print(f"wrote src/ui/brand_assets.rs with {len(entries)} marks")


if __name__ == "__main__":
    main()
