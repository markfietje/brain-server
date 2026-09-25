declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf8"): string;
  export function writeFileSync(path: string, data: string, encoding?: "utf8"): void;
  export function mkdtempSync(prefix: string): string;
  type RemoveOptions = {
    recursive: boolean | undefined;
    force: boolean | undefined;
  };
  export function rmSync(path: string, ...options: [] | [RemoveOptions]): void;

  const fs: {
    readFileSync: typeof readFileSync;
    writeFileSync: typeof writeFileSync;
    mkdtempSync: typeof mkdtempSync;
    rmSync: typeof rmSync;
  };
  export default fs;
}

declare module "node:os" {
  export function tmpdir(): string;
  const os: { tmpdir: typeof tmpdir };
  export default os;
}

declare module "node:path" {
  export function join(...parts: string[]): string;
  const path: { join: typeof join };
  export default path;
}

declare const process: {
  env: Record<string, string | undefined>;
};

declare module "typebox" {
  export type SchemaOptions = {
    description?: string;
    default?: unknown;
    minimum?: number;
    maximum?: number;
  };

  export interface TSchema {
    readonly __typebox?: never;
  }

  export interface TOptionalSchema<Inner extends TSchema> extends TSchema {
    readonly __typeboxOptional: Inner;
  }

  export interface TObjectSchema<Properties extends Record<string, TSchema>> extends TSchema {
    readonly __typeboxObject: Properties;
  }

  export interface TArraySchema<Item extends TSchema> extends TSchema {
    readonly __typeboxArray: Item;
  }

  export interface TUnionSchema<Members extends ReadonlyArray<TSchema>> extends TSchema {
    readonly __typeboxUnion: Members;
  }

  export interface TRecordSchema<Key extends TSchema, Value extends TSchema> extends TSchema {
    readonly __typeboxRecord: readonly [Key, Value];
  }

  export interface TLiteralSchema<Value extends string | number | boolean | null> extends TSchema {
    readonly __typeboxLiteral: Value;
  }

  export interface TStringSchema extends TSchema {
    readonly __typeboxString: true;
  }

  export interface TNumberSchema extends TSchema {
    readonly __typeboxNumber: true;
  }

  export interface TBooleanSchema extends TSchema {
    readonly __typeboxBoolean: true;
  }

  export type TObject = TObjectSchema<Record<string, TSchema>>;

  type OptionalPropertyNames<Properties extends Record<string, TSchema>> = {
    [Key in keyof Properties]: Properties[Key] extends TOptionalSchema<TSchema> ? Key : never;
  }[keyof Properties];

  type RequiredPropertyNames<Properties extends Record<string, TSchema>> = Exclude<
    keyof Properties,
    OptionalPropertyNames<Properties>
  >;

  export type Static<Schema extends TSchema> = Schema extends TOptionalSchema<infer Inner>
    ? Static<Inner> | undefined
    : Schema extends TObjectSchema<infer Properties>
      ? {
          [Key in RequiredPropertyNames<Properties>]: Static<Properties[Key]>;
        } & {
          [Key in OptionalPropertyNames<Properties>]?: Static<Properties[Key]>;
        }
      : Schema extends TArraySchema<infer Item>
        ? Array<Static<Item>>
        : Schema extends TUnionSchema<infer Members>
          ? Static<Members[number]>
          : Schema extends TRecordSchema<infer Key, infer Value>
            ? Record<Extract<Static<Key>, string>, Static<Value>>
            : Schema extends TLiteralSchema<infer Value>
              ? Value
              : Schema extends TStringSchema
                ? string
                : Schema extends TNumberSchema
                  ? number
                  : Schema extends TBooleanSchema
                    ? boolean
                    : never;

  export type TypeBuilder = {
    Optional<Schema extends TSchema>(schema: Schema): TOptionalSchema<Schema>;
    String(options?: SchemaOptions): TStringSchema;
    Number(options?: SchemaOptions): TNumberSchema;
    Integer(options?: SchemaOptions): TNumberSchema;
    Boolean(options?: SchemaOptions): TBooleanSchema;
    Literal<Value extends string | number | boolean | null>(value: Value): TLiteralSchema<Value>;
    Array<Item extends TSchema>(item: Item, options?: SchemaOptions): TArraySchema<Item>;
    Union<Members extends ReadonlyArray<TSchema>>(members: Members): TUnionSchema<Members>;
    Record<Key extends TSchema, Value extends TSchema>(
      key: Key,
      value: Value,
      options?: SchemaOptions,
    ): TRecordSchema<Key, Value>;
    Object<Properties extends Record<string, TSchema>>(
      properties: Properties,
      options?: SchemaOptions,
    ): TObjectSchema<Properties>;
  };

  export const Type: TypeBuilder;
}

declare module "typebox/value" {
  import type { Static, TSchema } from "typebox";

  export function Check<Schema extends TSchema>(schema: Schema, value: unknown): value is Static<Schema>;
}
