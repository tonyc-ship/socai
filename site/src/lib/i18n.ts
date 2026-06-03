// Shared server-side i18n helpers. Each page owns its own message dictionary
// (so the embedded `#site-i18n` JSON the client re-applies stays page-local),
// but the lookup/interpolation logic lives here so it can't drift per page.

type Dictionary = Record<string, unknown>;
type Messages = Record<string, Dictionary>;

function resolve(messages: Messages, path: string, language: string): unknown {
    return path.split(".").reduce<unknown>((cursor, key) => {
        if (cursor && typeof cursor === "object") {
            return (cursor as Record<string, unknown>)[key];
        }
        return undefined;
    }, messages[language]);
}

export function message(
    messages: Messages,
    path: string,
    language: string,
): string {
    const value = resolve(messages, path, language);
    return typeof value === "string" ? value : "";
}

export function formatMessage(
    messages: Messages,
    path: string,
    values: Record<string, string | number> = {},
    language: string,
): string {
    return message(messages, path, language).replace(
        /\{(\w+)\}/g,
        (_match, key) => String(values[key] ?? ""),
    );
}
