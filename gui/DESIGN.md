# The interface

AniRust is a desktop client for Anixart, and its interface is meant to read as
the phone client on a bigger screen rather than as a Material application that
happens to show anime. The direction, in five lines:

- **Product.** Browse Anixart's catalogue, open a release, watch it — for
  people who already know the phone client.
- **Character.** Warm, direct, unhurried. Nothing is dense for the sake of
  fitting more on screen.
- **World.** A home cinema and a poster wall: paper-white surfaces, coral ink,
  a matte dark screen while something plays.
- **Moments.** Colour belongs to posters and stills. On a release there is one
  coral button, and it is the one that starts the episode.
- **Signature.** The soft coral capsule behind the chosen destination, and the
  status pill with a chevron — the same pair the phone client uses.

## Tokens

| Token | Where | Value in spirit |
| --- | --- | --- |
| Accent | `globals/scheme.slint` | Anixart coral `#DC3540` (a shade deeper than the phone's `#FF4B55`, so white on it clears 4.5:1) |
| Surfaces | `globals/scheme.slint` | White page, `#F7F7F8` background, neutral steps up to `#E5E5EA` |
| Soft washes | `globals/tint.slint` | Coral, blue, mint, amber and violet pastels, each with ink that clears 4.5:1 on its wash |
| Radii | `globals/shape.slint` | Controls are pills; cards and posters 20px; sheets 28px |
| Working column | `pages/home.slint`, `main.slint` | The catalogue fills the page beside the rail and adds columns as the window grows |
| Spacing | `globals/shape.slint` | 4px grid, page edges and section gaps 24–28px |
| Type | `globals/typography.slint` | Inter; headings 600, body 400, labels 500 |

Light is the default only in the sense that following the desktop is: the
appearance defaults to the system's own. Dark and AMOLED are the same roles
with different surfaces; nothing else switches.

## Components

`components/ui-kit.slint` holds what the phone client is built from and the
component set does not spell the same way:

- `TintPill` — a pastel pill with an icon and, sometimes, a chevron.
- `TextTabs` — labels with a short coral underline under the chosen one.
- `SearchField` — the round field with a magnifier; Enter searches.
- `RoundIconButton` — a hairline circle for an action that needs no label.
- `Field` — a label above a rounded input, and a line of explanation below.
- `EmptyState` — one quiet icon, a sentence, and the action that changes it.
- `SectionTitle`, `InfoRow` — headings with a quiet action, and rows for
  settings.

`@material` still supplies the pieces that are complex rather than visual:
scroll views, switches, segmented controls, popups, and the player's chrome.

## Checking a screen

`cargo run -p anirust-gui --example screenshot -- out.png 1440 900 <state>`
renders the real components headlessly; `ANIRUST_SHOT_APPEARANCE=light|dark`
puts any state in that appearance. The state names are listed at the top of
`examples/screenshot.rs`. Layout mistakes are much cheaper to see in one of
those PNGs than in the source.
