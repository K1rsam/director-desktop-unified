export declare const OUTPUT_FORMATS: readonly ["table", "plain", "json", "yaml", "md", "csv"];
/** Validate before command execution and retain the renderer's existing aliases. */
export declare function normalizeOutputFormat(value: string): string;
