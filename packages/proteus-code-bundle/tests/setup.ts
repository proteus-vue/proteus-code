/**
 * Test setup shared by every suite.
 *
 * React reads `IS_REACT_ACT_ENVIRONMENT` when deciding whether to warn about
 * `act(...)`. It must be a `globalThis` property set before React's module body
 * runs, which is exactly what a setup file provides — a config `env` entry sets
 * `process.env` instead and React never sees it.
 */
;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
