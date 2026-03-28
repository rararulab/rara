# Rara Landing Design System

## Product Intent
- Positioning: GitHub Pages static promo for Rara
- Audience: developers who prefer terminal-first workflows
- Brand tone: warm, literary precision — like a beautifully typeset man page
- Language: English only, no CJK mixing

## Visual Direction
- Palette: Rosé Pine Dawn
- Atmosphere: warm parchment, high readability, low decorative noise
- Layout: narrow single-column documentation style (960px max)
- Surface language: monospaced typography, subtle warm borders, no gradients

## Design Tokens
### Color Roles (Rosé Pine Dawn)
- `base`: `#faf4ed` — page background
- `surface`: `#fffaf3` — card/panel backgrounds
- `overlay`: `#f2e9e1` — code blocks, hover states
- `muted`: `#9893a5` — secondary text, meta
- `subtle`: `#797593` — tertiary text
- `text`: `#575279` — primary text, headings
- `love`: `#b4637a` — primary accent, links
- `pine`: `#286983` — terminal commands, code keywords
- `foam`: `#56949f` — secondary interactive
- `iris`: `#907aa9` — tags, badges
- `gold`: `#ea9d34` — status indicators (sparingly)
- `rose`: `#d7827e` — hover states, secondary highlights

### Typography
- Font: JetBrains Mono (400, 500, 700)
- H1: 2.2rem, weight 700
- H2: 1.05rem, weight 700
- Body: 16px, line-height 1.7
- Meta/Nav: 0.82rem–0.85rem

### Shape and Spacing
- Panel radius: 10px
- Content max width: 960px
- Border weight: 1px
- Section gap: 2.5rem

## Motion
- Hover color transitions (150ms ease) only
- No animations, no floating, no scan lines

## Page Structure
1. Header: brand + plain text nav, border-bottom separator
2. Hero: kicker, headline, subtitle, text-link CTAs
3. Quick Start: code block on overlay background
4. Core Modules: 2×2 card grid with crate badges
5. Architecture Flow: centered monospace pipeline
6. Footer: minimal meta strip with border-top

## Content Rules
- English only throughout
- Short, direct copy — no marketing fluff
- Command blocks for all operational examples
- Lowercase headings in module cards
