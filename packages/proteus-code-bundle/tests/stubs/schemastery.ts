/**
 * Test stub for `@deepseek-ai/schemastery`.
 *
 * Records nothing and performs no validation: the tests exercise registration
 * behavior, not schema validation, which the harness runs during plugin load.
 */

interface Chain<T> {
  default(value: T): unknown
}

function chain<T>(): Chain<T> & Record<string, unknown> {
  const self = {
    default: () => self,
  } as Chain<T> & Record<string, unknown>
  return self
}

const Schema = {
  object: (fields: unknown) => ({ kind: 'object', fields }),
  string: () => chain(),
  number: () => chain(),
  boolean: () => chain(),
  any: () => ({}),
}

export default Schema
