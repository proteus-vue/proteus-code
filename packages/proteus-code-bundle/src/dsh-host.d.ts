/**
 * Ambient declarations for the minimal DeepSeek Harness host surface this
 * bundle touches.
 *
 * The published DSH package graph is not self-consistent yet (several peer
 * targets such as `@deepseek-ai/dsh-type-meta` are unpublished, and package
 * versions do not line up), so declaring DSH packages as dependencies would
 * make `pnpm install` fail. Instead the bundle keeps its DSH imports
 * runtime-only — they resolve from the running DSH installation's module
 * fallback — and type-checks against these local declarations.
 *
 * These mirror only what this package uses. When the upstream packages publish
 * a consistent graph, replace this file with real `devDependencies`.
 */

declare module '@deepseek-ai/cordis' {
  /** Scoped registration context handed to an `inject` callback. */
  export interface ScopedContext {
    [key: string]: unknown
  }

  /** The plugin context. Only the members this bundle uses are declared. */
  export interface Context {
    /** Register a follow-up plugin gated on the named services. */
    inject(deps: string[], callback: (scope: Context) => unknown): unknown
    /** Look up a mounted service by name, or `undefined` when absent. */
    get(name: string): unknown
    /**
     * Subscribe to a framework event. The registration is an effect, so it is
     * removed when the plugin unloads. `tools/pre-execute` is a waterfall: the
     * listener must call `next()` to delegate to the rest of the chain.
     */
    on(event: string, listener: (...args: never[]) => unknown): unknown
  }
}

declare module '@deepseek-ai/schemastery' {
  /** A validating schema with a static output type. */
  interface Schema<T> {
    readonly __type?: T
  }
  interface SchemaBuilder<T> {
    default(value: T): Schema<T>
  }
  interface ObjectSchemaFactory {
    <T extends Record<string, unknown>>(fields: {
      [K in keyof T]: Schema<T[K]> | SchemaBuilder<T[K]>
    }): Schema<T>
  }
  const Schema: {
    object: ObjectSchemaFactory
    string(): SchemaBuilder<string> & Schema<string>
    number(): SchemaBuilder<number> & Schema<number>
    boolean(): SchemaBuilder<boolean> & Schema<boolean>
    /** Unconstrained value, for opaque config the plugin validates itself. */
    any(): Schema<unknown>
  }
  export default Schema
}

declare module '@deepseek-ai/dsh-tools' {
  /** One model-facing content block. */
  export interface TextBlock {
    type: 'text'
    text: string
  }
  export type ContentBlock = TextBlock | { type: string; [key: string]: unknown }

  /** Annotation fields shared by every schema node. */
  interface SchemaAnnotations {
    description?: string
    title?: string
    default?: unknown
    examples?: unknown
  }
  export interface StringSchemaSpec extends SchemaAnnotations {
    type: 'string'
    enum?: readonly string[]
    const?: string
  }
  export interface NumberSchemaSpec extends SchemaAnnotations {
    type: 'number'
  }
  export interface IntegerSchemaSpec extends SchemaAnnotations {
    type: 'integer'
  }
  export interface BooleanSchemaSpec extends SchemaAnnotations {
    type: 'boolean'
  }
  export interface NullSchemaSpec extends SchemaAnnotations {
    type: 'null'
  }
  export interface ArraySchemaSpec extends SchemaAnnotations {
    type: 'array'
    items?: ValueSchemaSpec
  }
  export interface ObjectSchemaSpec extends SchemaAnnotations {
    type: 'object'
    properties?: ParameterSchemaSpec
    additionalProperties: boolean
  }
  export interface OneOfSchemaSpec extends SchemaAnnotations {
    oneOf: readonly ValueSchemaSpec[]
  }
  export type ValueSchemaSpec =
    | StringSchemaSpec
    | NumberSchemaSpec
    | IntegerSchemaSpec
    | BooleanSchemaSpec
    | NullSchemaSpec
    | ArraySchemaSpec
    | ObjectSchemaSpec
    | OneOfSchemaSpec

  /** One implicit parameter-root property, optionally required. */
  export type ParameterPropertySpec = ValueSchemaSpec & { required?: true }
  /** The implicit open object root of a tool's parameters. */
  export type ParameterSchemaSpec = Record<string, ParameterPropertySpec>

  /** Canonical output declaration plus its model-facing projection. */
  export interface ToolOutputDefinition {
    schema: ValueSchemaSpec
    render(args: unknown, value: unknown): ContentBlock[]
    presentationMeta?(args: unknown, value: unknown): unknown
  }

  /** Immutable execution identity and cancellation signal. */
  export interface ToolRunContext {
    readonly signal: AbortSignal
    readonly agent?: unknown
    readonly [key: string]: unknown
  }

  /** A registered tool: schema plus execution function. */
  export interface ToolDefinition {
    readonly name: string
    readonly description: string
    readonly parameters: ParameterSchemaSpec
    readonly output: ToolOutputDefinition
    execute(args: unknown, exec: ToolRunContext): Promise<unknown>
  }

  /** Options accepted by {@link defineTool}. */
  export interface DefineToolOptions<S extends ParameterSchemaSpec, O extends ValueSchemaSpec> {
    name: string
    description: string
    parameters: S
    output: {
      schema: O
      render(args: InferArgs<S>, value: unknown): ContentBlock[]
      presentationMeta?(args: InferArgs<S>, value: unknown): unknown
    }
    execute(args: InferArgs<S>, exec: ToolRunContext): Promise<unknown>
    timeoutMs?: number
  }

  /** Infer the type-safe argument object from a parameter schema spec. */
  export type InferArgs<S extends ParameterSchemaSpec> = {
    [K in keyof S as S[K] extends { required: true } ? K : never]: InferValue<S[K]>
  } & {
    [K in keyof S as S[K] extends { required: true } ? never : K]?: InferValue<S[K]>
  }

  /** Infer a TypeScript type from one value schema spec. */
  export type InferValue<S> = S extends { type: 'string' }
    ? string
    : S extends { type: 'number' | 'integer' }
      ? number
      : S extends { type: 'boolean' }
        ? boolean
        : S extends { type: 'null' }
          ? null
          : S extends { type: 'array'; items: infer I }
            ? InferValue<I>[]
            : S extends { type: 'object' }
              ? Record<string, unknown>
              : unknown

  /** Declare a tool with inferred, validated arguments. */
  export function defineTool<S extends ParameterSchemaSpec, O extends ValueSchemaSpec>(
    options: DefineToolOptions<S, O>,
  ): ToolDefinition

  declare module '@deepseek-ai/cordis' {
    interface Context {
      tools: {
        register(definition: ToolDefinition): () => void
      }
    }
  }
}
