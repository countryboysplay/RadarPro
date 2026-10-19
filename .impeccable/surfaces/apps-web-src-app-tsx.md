---
version: 1
slug: "apps-web-src-app-tsx"
primary_target: "apps/web/src/App.tsx"
related_targets: ["apps/web/src/App.css","apps/web/src/map/MapView.tsx"]
---

# Surface: RadarPro workstation (apps/web/src/App.tsx)

Mode: Operate. Audience: storm chasers on a 13-15" laptop in a moving vehicle, often at night.
Task: read a storm fast. Hot path: site, product, tilt, loop. Never behind a menu.
Constraints: map dominant, dark theme, every feature reachable, forecast data never in the sidebar
(it rides the time strip beyond NOW). Owner favourite colour #739BD0 is the shell accent.

## Direction contract

THESIS: The radar is terrain: fixed, never edited, read like an orienteering map. Everything the
chaser adds (cross-section leg, probe, chosen site) rides above it in one course ink. Refuses the
category default of a grey toolbar of buttons over a map.

OWN-WORLD: Night ISOM. Ground #0e1110 / panels #161b19 / rules #262e2a, paper-white ink #ecebe4,
muted #a3a89f. Course ink purple #b56cff with a dark casing, used only for user marks. Shell accent
#739BD0 for selection and primary action. Condensed sans (Roboto Condensed) at real sizes; tabular
numerals; ranked legend tables; control circles, start triangle, finish double circle as the only
pictograms.

STORY: The chaser sees the site, product, tilt and scan time at a glance, flips product and tilt in
one click, loops scans, and marks a leg across a storm to slice it.

FIRST VIEWPORT: Left column 300 px: wordmark, site (code grid search), product segment, tilt
segment, then course table (leg + probe), warnings in view, and collapsed Layers / Tools /
Settings. Map fills the rest. Bottom strip 64 px: transport, scan ticks to NOW, forecast hatched
beyond NOW with its model/variable picker.

FORM: orienteering map and legend, challenger (notation-diagram-systems-orienteering-map), seed:2745c92188fa82a0.
Signature interaction: the cross-section is set like a course leg: start triangle, purple leg,
finish double circle drawn on the map, with its description row (length, bearing) in the course
table. Motion: the leg draws from start to finish (200 ms); nothing else animates.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

## Unresolved

- Mobile layout: the left column collapses to a bottom sheet under 900 px wide (secondary; laptop first).
