# RocoKnight
RocoKnight - Roco Kingdom client shell (Ruffle + Electron)

## Overview
A minimal Electron shell that loads a local SWF via Ruffle. This is a base runtime only; it does not include any game logic or automation.

## Requirements
- Node.js 18+
- A locally available Ruffle web build
- An authorized SWF file placed at `assets/game.swf`

## Setup
1. Install dependencies:

```bash
npm install
```

2. Download Ruffle Web (self-host build) and place files into `vendor/ruffle/`:

Expected:
- `vendor/ruffle/ruffle.js`
- `vendor/ruffle/ruffle.wasm`

Example (download manually):
- https://ruffle.rs/#downloads

3. Place your SWF at `assets/game.swf`.

## Run
```bash
npm run dev
```

## Notes
- AS3 compatibility depends on Ruffle's AVM2 support and may be incomplete.
- This shell is intentionally minimal for a clean base.
