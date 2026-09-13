/// <reference types="vite/client" />

// S09b: first Vite env var this project uses (see .env.example). Typed
// explicitly so `import.meta.env.VITE_RAINBOW_API_KEY` is checked, and so
// it reads as `string | undefined` (unset/empty in every build that has no
// `.env.local`) rather than `any` at every call site.
interface ImportMetaEnv {
  readonly VITE_RAINBOW_API_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
