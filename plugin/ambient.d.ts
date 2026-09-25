declare module "openclaw/plugin-sdk/plugin-entry" {
  export type PluginHookContext = {
    agentId?: string;
    sessionKey?: string;
    sessionId?: string;
    channel?: string;
    channelId?: string;
    chatId?: string;
    chatType?: "direct" | "group" | "channel" | "explicit";
    trigger?: string;
  };

  export type BeforePromptBuildEvent = {
    prompt: string;
    messages: ReadonlyArray<unknown>;
  };

  export type AgentEndEvent = {
    success: boolean;
    messages: ReadonlyArray<unknown>;
  };

  export type BeforeAgentRunEvent = {
    prompt?: string;
  };

  export type SessionEndEvent = Readonly<Record<string, never>>;

  export type HookResult = {
    prependContext?: string;
    prependSystemContext?: string;
    appendContext?: string;
  };

  export type PluginHookEventMap = {
    before_prompt_build: BeforePromptBuildEvent;
    agent_end: AgentEndEvent;
    before_agent_run: BeforeAgentRunEvent;
    session_end: SessionEndEvent;
  };

  export type PluginHookHandler<Name extends keyof PluginHookEventMap> = (
    event: PluginHookEventMap[Name],
    context: PluginHookContext | undefined,
  ) => HookResult | void | Promise<HookResult | void>;

  export type PluginLogger = {
    info(message: string): void;
    warn(message: string): void;
    debug?(message: string): void;
    error?(message: string): void;
  };

  export type PluginRuntimeConfig = {
    plugins?: {
      entries: Record<string, { config: unknown }> | undefined;
    };
  };

  export type MemoryCorpusSearchArgs = {
    query: string;
    maxResults?: number;
    agentId?: string;
    sandboxed?: boolean;
  };

  export type MemoryCorpusGetArgs = {
    lookup: string;
  };

  export type MemoryCorpusResult = Readonly<Record<string, unknown>>;

  export type MemoryCorpusRegistration = {
    search(
      args: MemoryCorpusSearchArgs,
    ): Promise<ReadonlyArray<MemoryCorpusResult>>;
    get(args: MemoryCorpusGetArgs): Promise<MemoryCorpusResult | null>;
  };

  export type MemoryCapabilityRegistration = {
    promptBuilder(): ReadonlyArray<string>;
  };

  export type PluginToolContent = Readonly<{
    type: "text";
    text: string;
  }>;

  export type PluginToolResult = {
    content: ReadonlyArray<PluginToolContent>;
    details?: unknown;
  };

  export type PluginToolExecute = (
    toolCallId: string,
    params: unknown,
  ) => PluginToolResult | Promise<PluginToolResult>;

  export type PluginToolRegistration = {
    name: string;
    label?: string;
    description?: string;
    parameters?: unknown;
    execute: PluginToolExecute;
  };

  export type PluginServiceRegistration = {
    id: string;
    start(): Promise<void>;
    stop(): void;
  };

  export interface OpenClawPluginApi {
    readonly pluginConfig: unknown;
    readonly logger: PluginLogger;
    readonly runtime: {
      config: {
        current(): PluginRuntimeConfig;
      };
    };
    resolvePath(path: string): string;
    on<Name extends keyof PluginHookEventMap>(
      name: Name,
      handler: PluginHookHandler<Name>,
      ...options: [] | [{ timeoutMs: number | undefined }]
    ): void;
    registerTool(
      tool: PluginToolRegistration,
      ...options: [] | [{ name: string | undefined }]
    ): void;
    registerService(service: PluginServiceRegistration): void;
    registerMemoryCapability(capability: MemoryCapabilityRegistration): void;
    registerMemoryCorpusSupplement(corpus: MemoryCorpusRegistration): void;
  }

  export type PluginEntry = {
    id: string;
    name: string;
    description: string;
    configSchema: unknown;
    register(api: OpenClawPluginApi): void;
  };

  export function definePluginEntry(entry: PluginEntry): PluginEntry;
}
