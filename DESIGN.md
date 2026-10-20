---
name: RadarPro
description: A night orienteering map for storm chasers; the radar is terrain, the chrome is its legend.
colors:
  ground-deep: "#0b0e0d"
  ground: "#0e1110"
  panel: "#161b19"
  raised: "#1d2420"
  rule: "#262e2a"
  rule-strong: "#36403a"
  ink-faint: "#858b80"
  ink-muted: "#a3a89f"
  paper-ink: "#ecebe4"
  accent: "#739bd0"
  accent-strong: "#8fb0dc"
  accent-ink: "#0b0e0d"
  course-ink: "#b56cff"
  forecast-amber: "#f0b84a"
  ok-green: "#8fcf8a"
  danger-red: "#ef6a5a"
typography:
  display:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "34px"
    fontWeight: 800
    lineHeight: 0.95
    letterSpacing: "-0.01em"
    fontFeature: "tnum"
  headline:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "22px"
    fontWeight: 800
    lineHeight: 1
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 700
    lineHeight: 1.2
    fontFeature: "tnum"
  control:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.02em"
  body:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "tnum"
  label:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.08em"
  meta:
    fontFamily: "Roboto Condensed Variable, Roboto Condensed, Arial Narrow, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.35
    fontFeature: "tnum"
rounded:
  hairline: "1px"
  chip: "2px"
  base: "3px"
  dot: "50%"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
  rail: "320px"
  strip: "64px"
components:
  button:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.paper-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.base}"
    padding: "0 12px"
    height: "30px"
  button-hover:
    backgroundColor: "{colors.rule}"
  button-disabled:
    backgroundColor: "transparent"
    textColor: "{colors.ink-faint}"
  segment:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.paper-ink}"
    typography: "{typography.control}"
    rounded: "{rounded.base}"
    padding: "0 4px"
    height: "36px"
  segment-selected:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
  button-play:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
    rounded: "{rounded.base}"
    padding: "0 8px"
    height: "36px"
    width: "46px"
  button-play-hover:
    backgroundColor: "{colors.accent-strong}"
  input:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.paper-ink}"
    rounded: "{rounded.base}"
    padding: "0 8px"
    height: "30px"
  category-header:
    backgroundColor: "transparent"
    textColor: "{colors.ink-muted}"
    typography: "{typography.label}"
    height: "44px"
  course-row:
    textColor: "{colors.paper-ink}"
    typography: "{typography.body}"
    padding: "7px 4px"
  forecast-toggle:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.forecast-amber}"
    rounded: "{rounded.base}"
    padding: "0 14px"
    height: "46px"
  time-strip:
    backgroundColor: "{colors.panel}"
    height: "{spacing.strip}"
    padding: "0 16px"
  rail:
    backgroundColor: "{colors.panel}"
    width: "{spacing.rail}"
    padding: "0 16px 24px"
---

# Design System: RadarPro

## Overview

**Creative North Star: "The Night Orienteering Map"**

The radar is terrain: fixed, never restyled, read the way an orienteer reads a printed map. Everything around it is the map's legend, set at night: paper-white ink on near-black green-greys, ranked tables with hairline rules, one condensed sans at sizes you can read in a moving vehicle. Whatever the chaser places on the terrain (the cross-section leg, the chosen site, the probe's control) is printed in one course ink. It sits above the echoes on a dark casing, the way an orienteering course is overprinted on a map.

The workstation is dense but calm. A 320px rail on the left holds the hot path (site, product, tilt, course) open at all times. Depth lives in collapsed categories below it: Warnings, SPC, My location, Layers, Archive. Set-once options (storms and lightning, location alerts, placefiles, color tables, data) live in a Settings window opened from the rail head. A single 64px time strip runs along the bottom: transport, one time axis with observed scans up to NOW, then forecast hours hatched in amber beyond it. The map gets everything else. Only two colours carry meaning in the chrome. Steel blue marks what is selected and the one primary action. Amber marks model time. Everything else is a neutral from one stepped ramp.

The world rejects the category default of a grey toolbar of buttons floating over the map. It also rejects chrome that covers the radar, controls hidden behind menus, and any presentation that lets a forecast pass for an observation.

**Key Characteristics:**
- Map dominant: fixed 320px rail and 64px strip; the map takes the rest.
- One condensed family, tabular numerals everywhere.
- Flat legend chrome: 1px rules and tonal steps, not cards and shadows.
- Two reserved inks (steel-blue accent, course purple) plus amber for model time.
- Orienteering pictograms (start triangle, control circle, finish double circle) shared by the rail and the map.
- Every displayed time is 12-hour local.

## Colors

A nine-step green-grey night ramp carries the chrome. Steel blue, course purple and forecast amber each have one job.

### Primary
- **Steel Blue Accent** (#739bd0): selection and the primary action, nothing else. It fills the selected product and tilt segments, the play button, the active observed tick on the time axis, slider thumbs and the focus ring. It is also the text-selection colour. Text on it is Accent Ink (#0b0e0d).
- **Lifted Steel** (#8fb0dc): hover on the play button and link text in the rail. It is never used as a fill at rest.

### Secondary
- **Course Purple** (#b56cff): marks the user has placed on the terrain. Those are the cross-section leg (start triangle, leg line, finish double circle), the selected radar-site dot, the course-table pictograms and the "pick a point" hint. On the map it is always drawn over a 7px casing in Ground (#0e1110), with the 3px ink stroke on top.

### Tertiary
- **Forecast Amber** (#f0b84a): model time beyond NOW. It covers forecast ticks, the 10% hatch on the forecast side of the time track, the forecast source label, the Forecast toggle in the strip, the forecast drawer's 2px top rule and heading, and the 3px left rule on the map's "Forecast radar ... model, not observed" badge.

### Neutral
- **Ground Deep** (#0b0e0d): behind the map and the canvases (cross-section, forecast field).
- **Ground** (#0e1110): page ground, input fields, alert detail wells, and the course casing.
- **Panel** (#161b19): the rail, the time strip, the forecast drawer, floating panels.
- **Raised** (#1d2420): button and segment fill at rest, the observed half of the time track, hovered or selected alert rows.
- **Rule** (#262e2a): every 1px divider: rail blocks, category edges, course-table rows, panel borders. Also button hover.
- **Rule Strong** (#36403a): button and input borders, scrollbar thumbs, unpublished forecast stubs.
- **Ink Faint** (#858b80): notes, empty states, definition-list terms, timeline labels, disabled text, chevrons.
- **Ink Muted** (#a3a89f): rail headings, the site name, secondary lines, observed ticks, range rings and unselected site dots on the map.
- **Paper Ink** (#ecebe4): primary text, values, the NOW marker.

### Status
- **Live Green** (#8fcf8a): the live-feed dot and LIVE mode, plus ready statuses. It is never used for a control.
- **Danger Red** (#ef6a5a): feed errors and failed statuses.

### Named Rules
**The Two Reserved Inks Rule.** Steel blue means "selected or primary". Course purple means "the user put this here". Neither appears as decoration, as a category colour or as a data colour.

**The Amber Is Model Time Rule.** Amber marks forecast and model output beyond NOW, so it can never read as observed. When a new state needs a warning colour, it does not borrow amber.

**The One Ramp Rule.** Neutrals come only from the nine ground-to-paper steps. A new surface picks a step; it does not mix a new grey.

## Typography

**Display Font:** Roboto Condensed Variable (self-hosted via @fontsource-variable, falling back to Roboto Condensed, Arial Narrow, system-ui)
**Body Font:** the same family
**Label/Mono Font:** the same family with tabular numerals; ui-monospace only inside the colour-table editor's JSON textarea

**Character:** One condensed grotesque doing every job, like the type on a printed orienteering legend. It is narrow enough to fit a full product row in 320px and heavy enough to read through glare. Hierarchy comes from weight and size steps, never from a second family.

### Hierarchy
- **Display** (800, 34px, 0.95): the site code (KTLX) only. It is the largest thing in the chrome.
- **Headline** (800, 22px, 1, uppercase): the RADARPRO wordmark in the sticky rail head.
- **Title** (700, 18px): the current scan or valid time in the strip. Drawer and alert-detail titles step down to 15-17px at 700.
- **Control** (700, 16px, +0.02em): product segment labels; tilt segments are 15px.
- **Body** (400, 14px, 1.35): default text, buttons (at 600), course-table rows (values at 700). Long alert text is 13px/1.45, capped at 65ch.
- **Label** (700, 13px, +0.08em, uppercase): rail block headings (SITE, PRODUCT, TILT, COURSE) and category headers, in Ink Muted. Each one is the real heading of the control group below it.
- **Meta** (400, 12px): ages, axis notes, legend domain, statuses. Timeline end labels and attributions go down to 11px; the NOW tag is 10px/700/+0.12em.

### Named Rules
**The Tabular Rule.** Body text sets `tabular-nums` globally. Times, ranges, bearings and dBZ never shift width as they change.

**The 12-Hour Local Rule.** Every time the chrome formats is 12-hour local (`hour12: true`), for example "11:39:38 AM" or "Oct 19, 1:00 PM". Where room is tight (the strip's forecast label), a time that falls today drops its date ("valid 12:00 PM"). Bearings are three digits with a degree sign ("060°"). Distances use one decimal with the unit ("255.0 km").

## Layout

The layout is a fixed full-viewport grid. A **320px rail** spans the full height on the left. The **map** fills the remaining width. A **64px time strip** runs under the map only, so the rail keeps its full height. The rail scrolls on its own, and its head (wordmark, feed status, settings button) is sticky. The Settings window floats over the map's top-left corner, 680px wide, with section tabs on its left, and closes with × or Esc. It does not dim the map, so each change shows on the radar while it is open.

- **Rail rhythm:** 16px side padding. Hot-path blocks have 18px above and 16px below, each closed by a 1px Rule. Headings sit 8px above their controls. Collapsible categories have 44px headers and 12px gaps between children; nested sections have 38px headers.
- **Segments:** product is a 4-column grid and tilt is 5 columns, both with 4px gaps. Every option is visible at once, so nothing hides in a menu.
- **Strip:** three columns (transport and status, time axis, Forecast toggle) with 20px gaps. The axis puts NOW at a fixed 35% of the track: observed time to the left, forecast time to the right.
- **Map overlays:** they float 12px from the map edges. The forecast drawer is at most 380px wide (bottom-right), the point-forecast panel at most 360px (top-right), and the forecast badge sits top-left. Each can be dismissed.
- **Spacing scale:** 4 / 8 / 12 / 16 / 24px, with 6, 10 and 14px used for in-component nudges.
- **Under 900px:** the grid becomes one column: the map at 60vh, then the strip (auto height, stacked), then the rail in document flow. Laptop is the primary target, and phones get this fallback only.

**The Map Dominant Rule.** Chrome lives in the rail and the strip. Anything drawn over the map must be small, dismissable, and anchored to a corner.

**The Forecast Lives With Time Rule.** Model fields and point forecasts open from the time strip, beyond NOW. They never appear in the rail.

## Elevation & Depth

The chrome is flat. Depth comes from tonal steps on the night ramp (Ground, then Panel, then Raised) and from 1px rules, the way a printed legend separates rows. Shadows exist only on elements that float over the map, where they lift the chrome off the terrain.

### Shadow Vocabulary
- **Float** (`box-shadow: 0 8px 24px rgb(0 0 0 / 0.45), 0 2px 6px rgb(0 0 0 / 0.35)`): forecast drawer and right-hand point-forecast panel only.
- **Selection inset** (`box-shadow: inset 0 0 0 1px #739bd0`): selected alert row.
- **Tick halo** (`box-shadow: 0 0 0 2px #0e1110`): active tick on the time track, separating it from its neighbours.

### Named Rules
**The Flat Legend Rule.** Nothing in the rail or strip casts a shadow. When a surface needs separating, use a rule or a tonal step.

## Shapes

The form language is near-square. Controls, inputs, panels and the time track share one small radius (3px). Severity chips use 2px. Time ticks and the legend bar use 1px. Status and severity dots are full circles. Dividers are 1px rules in Rule (#262e2a), and floats over the map add a Panel border.

The only pictograms are one SVG set on a 20-unit grid with a 1.75 stroke and round caps and joins. It contains transport (prev, play, pause, next, latest), chevrons, close, star, a forecast trace, a settings slider pair, and the three course symbols: **start triangle**, **control circle** and **finish double circle**. The rail and the map draw the course symbols with the same geometry. On the map they are 3px course ink over a 7px casing: a 13px triangle and a finish circle of 11px and 6.5px radii. The leg stops short of each symbol instead of running through it.

## Components

### Buttons
Quiet slabs that read as part of the legend, not a toolbar.
- **Shape:** gently squared (3px), min height 30px, 0 12px padding, 600 14px type.
- **Default:** Raised fill, Rule Strong 1px border, Paper Ink text.
- **Hover / Focus:** the fill steps to Rule and the border to Ink Faint over 120ms ease-out. Focus is a 2px steel-blue outline with a 2px offset.
- **Disabled:** transparent fill, Rule border, Ink Faint text.
- **Quiet:** transparent fill (Clear, secondary actions beside a primary).
- **Primary (Play):** the one filled button: steel blue with Accent Ink, at least 46 by 36px, hover Lifted Steel.

### Segments (product, tilt, nearby sites)
The labels are the controls. Each is a 36px cell (32px for tilts) with Raised fill, no visible border, and 700 16px uppercase codes. The selected cell fills steel blue with Accent Ink. A favourite product shows as a 5px dot in the cell's top-right corner.

### Inputs / Fields
- **Style:** Ground fill, Rule Strong 1px border, 3px radius, min height 30px, 500 14px text. Labels and placeholders are Ink Muted and Ink Faint.
- **Focus:** the border turns steel blue, with the global focus outline at offset 0.
- **Sliders and checkboxes:** native controls with `accent-color` steel blue.

### Navigation (collapsible categories)
These are 44px full-width headers: a chevron, an uppercase Label title, and a right-aligned one-line live summary in Ink Faint (for example "306 US · 0 polygons near KTLX"). Hover lifts the title to Paper Ink. There is no fill and no card, only a 1px rule beneath.

### Course Table (signature)
This is the orienteering description sheet. Each row is ruled top and bottom in Rule, padded 7px 4px. A purple pictogram cell comes first: a control circle for the probe, start triangle plus finish double circle for the leg. A label follows, then right-aligned 700 tabular values (value with unit, "255.0 km", "060°"). The cross-section canvas sits beneath on Ground Deep with a 1px rule, and gaps between tilts stay empty.

### Course Leg on the Map (signature)
Setting a cross-section prints a course over the radar: start triangle aimed down the leg, leg line, finish double circle. All of it is course ink on its casing. The leg draws from start to finish over 200ms with an ease-out cubic. This is the system's one authored motion, and it is skipped under `prefers-reduced-motion`.

### Time Strip and Unified Axis (signature)
A single 64px row. The 24px track is Raised up to NOW (35%), then Ground with a 135° amber hatch (10% alpha, 5px on and 5px off). NOW is a 2px Paper Ink rule that extends 4px past the track, with a 10px tracked NOW tag above it. Observed ticks are 2px Ink Muted at 60% opacity. Forecast ticks are 2px amber at 55%. The active tick grows to 4px with a 2px Ground halo: steel blue when observed, amber when forecast. Unpublished forecast hours are short Rule Strong stubs and cannot be selected. Transport, mode (LIVE in green with a dot, PLAYING in steel blue) and the 18px time sit on the left. The amber-outlined Forecast toggle, at least 46px tall, sits on the right.

### Forecast Surfaces
- **Forecast drawer:** a Panel float with a 2px amber top rule, an amber uppercase 15px heading and the Float shadow. It opens from the strip toggle.
- **Forecast badge:** a Panel chip at the map's top-left with a 3px amber left rule. It reads "**FORECAST RADAR** HRRR simulated reflectivity · valid Mon 1:00 PM (+3h) · model, not observed", with a "Back to radar" button. It is shown whenever model radar replaces observed radar past NOW. For an HRRR severe-weather field (STP, CAPE, helicity, shear, LCL) it names the field instead and adds a 168px colour bar, labelled where colour starts, mid-scale and at the top with the unit; low values are clear so the map shows through.

### Sounding Panel
The HRRR sounding opens in the right panel from "Sounding at a point" in the Course block (the "pick a point" hint is course purple). Its valid time is amber, because it is always model output. Below it sit two plots on Ground Deep with a 1px Rule border, side by side at 3:2: a skew-T (1050-100 hPa, isotherms skewed 45°, Rule grid, 0 °C in Ink Faint) and a hodograph (20 kt rings). Course-table rows follow (surface T/Td, SBCAPE, LCL), then a Meta source line that says when the column is the coarse NOAA fallback.

The plot lines are data, like the radar, so they use their own skew-T colours and never the chrome inks: temperature #ff5a4d, dew point #3fbf5f, surface parcel Paper Ink dashed 4/3. The hodograph is banded by height above ground: 0-3 km #ff5a4d, 3-6 km #3fbf5f, 6-9 km #e6d94a and 9-12 km #4ab8e6. Those are deliberately not Danger Red, Live Green, Forecast Amber or Steel Blue.

### Map Marks
Radar sites are 4px Ink Muted dots with a 2px Ground stroke. The selected site is a 6px course-purple dot. Range rings are 1px Ink Muted at 50% alpha. The basemap is OpenFreeMap Dark. The radar, MRMS and forecast fields render through their own colour tables and are never tinted by the chrome.

## Do's and Don'ts

### Do:
- **Do** keep the rail at 320px and the time strip a single 64px row. The map takes every remaining pixel.
- **Do** keep site, product, tilt and transport visible at all times as segments and buttons, never inside a menu.
- **Do** use steel blue (#739bd0) only for the selected option, the primary action and focus.
- **Do** draw anything the user places on the radar in course purple (#b56cff) over a 7px Ground casing, using the start, control and finish pictograms.
- **Do** colour model time amber (#f0b84a) and label model radar on the map "model, not observed".
- **Do** format every displayed time as 12-hour local, set in tabular numerals.
- **Do** separate chrome with 1px rules and tonal steps, and save the Float shadow for panels that sit over the map.
- **Do** draw new icons on the 20-unit, 1.75-stroke, round-join grid in the Icon set.

### Don't:
- **Don't** restyle, tint or dim the radar terrain to suit the chrome.
- **Don't** put forecast or model data in the rail.
- **Don't** use course purple for selection or steel blue for user marks. The two inks never swap.
- **Don't** reuse amber for warnings, staleness or emphasis. It means model time.
- **Don't** add a second typeface, or proportional numerals in any readout.
- **Don't** let more than one loop play at once. Starting the forecast loop pauses the scan loop, and the reverse.
- **Don't** add motion beyond the 200ms leg draw and 120ms state-colour changes.
