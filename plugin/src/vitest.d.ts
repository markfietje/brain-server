declare module "vitest" {
  export interface Assertion {
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toContain(expected: unknown): void;
    toHaveLength(expected: number): void;
    toBeLessThan(expected: number): void;
    toBeLessThanOrEqual(expected: number): void;
    toBeGreaterThan(expected: number): void;
    toBeGreaterThanOrEqual(expected: number): void;
    toMatch(expected: string | RegExp): void;
    toMatchObject(expected: unknown): void;
    toHaveProperty(property: string, value?: unknown): void;
    toBeUndefined(): void;
    toBeNull(): void;
    toBeDefined(): void;
    toBeInstanceOf(expected: unknown): void;
    toThrow(expected?: unknown): void;
    toHaveBeenCalled(): void;
    toHaveBeenCalledTimes(expected: number): void;
    readonly not: Assertion;
    readonly resolves: Assertion;
    readonly rejects: Assertion;
  }

  export type MockInstance<Args extends readonly unknown[] = readonly unknown[], Result = void> = {
    (...args: Args): Result;
    mock: {
      calls: Args[];
      results: Result[];
      lastCall?: Args;
    };
    mockClear(): MockInstance<Args, Result>;
    mockReset(): MockInstance<Args, Result>;
    mockRestore(): MockInstance<Args, Result>;
    mockImplementation(implementation: (...args: Args) => Result): MockInstance<Args, Result>;
    mockReturnValue(value: Result): MockInstance<Args, Result>;
    mockResolvedValue(value: Awaited<Result>): MockInstance<Args, Result>;
    mockRejectedValue(value: unknown): MockInstance<Args, Result>;
    mockName(name: string): MockInstance<Args, Result>;
  };

  export type MockFunction<Args extends readonly unknown[] = readonly unknown[], Result = void> =
    MockInstance<Args, Result>;

  export type VitestApi = {
    fn<Args extends readonly unknown[] = readonly unknown[], Result = void>(
      implementation?: (...args: Args) => Result,
    ): MockFunction<Args, Result>;
    spyOn<Target extends object, Key extends keyof Target>(
      target: Target,
      method: Key,
    ): MockInstance<
      Target[Key] extends (...args: infer Args) => infer Result ? Args : never,
      Target[Key] extends (...args: infer Args) => infer Result ? Result : never
    >;
    stubGlobal(name: string, value: unknown): void;
    unstubAllGlobals(): void;
    restoreAllMocks(): void;
    clearAllMocks(): void;
  };

  export const vi: VitestApi;
  export type ExpectStatic = {
    (value: unknown, message?: string): Assertion;
    any(constructor: unknown): unknown;
    stringContaining(expected: string): unknown;
  };
  export const expect: ExpectStatic;
  export function describe(name: string, body: () => void): void;
  export function test(name: string, body: () => void | Promise<void>): void;
  export function afterEach(body: () => void | Promise<void>): void;
}
