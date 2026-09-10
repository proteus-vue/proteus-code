import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vitest/config'

const here = path.dirname(fileURLToPath(import.meta.url))

export default defineConfig({
  resolve: {
    // DSH host packages are runtime-only imports provided by the harness. Alias
    // them to local stubs so unit tests run without a DSH installation.
    alias: {
      '@deepseek-ai/dsh-tools': path.join(here, 'tests/stubs/dsh-tools.ts'),
      '@deepseek-ai/schemastery': path.join(here, 'tests/stubs/schemastery.ts'),
    },
  },
  test: {
    include: ['tests/**/*.test.ts'],
    environment: 'node',
    testTimeout: 30_000,
  },
})
