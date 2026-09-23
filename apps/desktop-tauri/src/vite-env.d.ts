/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_NEO_WORKSPACE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
