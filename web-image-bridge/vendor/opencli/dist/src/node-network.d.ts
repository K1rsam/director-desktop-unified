import { type Dispatcher } from 'undici';
export declare function hasProxyEnv(env?: NodeJS.ProcessEnv): boolean;
export declare function getNetworkDispatcher(env?: NodeJS.ProcessEnv): Dispatcher;
export declare function fetchWithNodeNetwork(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
export declare function installNodeNetwork(): void;
