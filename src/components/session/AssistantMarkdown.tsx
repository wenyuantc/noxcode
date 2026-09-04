import { Children, isValidElement, memo, useMemo, type ReactNode } from "react";
import type { Components } from "react-markdown";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { CodeBlock } from "@/components/code/CodeBlock";
import { languageFromClassName } from "@/lib/codeLanguage";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";

function markdownComponents(variant: "default" | "plan", codeFontSize: number): Components {
  const headingAccent = variant === "plan" ? "text-cyan-600 dark:text-cyan-400" : "";
  return {
    h1: ({ children }) => (
      <h1 className={cn("mb-2 text-base font-semibold", headingAccent)}>{children}</h1>
    ),
    h2: ({ children }) => (
      <h2 className={cn("mb-2 text-sm font-semibold", headingAccent)}>{children}</h2>
    ),
    h3: ({ children }) => (
      <h3 className={cn("mb-1 text-sm font-medium", headingAccent)}>{children}</h3>
    ),
    p: ({ children }) => <p className="mb-2 last:mb-0 leading-6">{children}</p>,
    ul: ({ className, children }) => {
      const task = className?.includes("contains-task-list");
      return (
        <ul className={cn("mb-2 space-y-1 last:mb-0", task ? "list-none pl-0" : "list-disc pl-5")}>
          {children}
        </ul>
      );
    },
    ol: ({ children }) => (
      <ol className="mb-2 list-decimal space-y-1 pl-5 last:mb-0">{children}</ol>
    ),
    li: ({ className, children }) => {
      const task = className?.includes("task-list-item");
      return (
        <li className={cn("leading-6", task && "flex list-none items-start gap-2")}>{children}</li>
      );
    },
    input: ({ type, checked }) => {
      if (type !== "checkbox") return null;
      return (
        <input
          type="checkbox"
          checked={Boolean(checked)}
          disabled
          readOnly
          className="mt-1.5 size-3.5 shrink-0 accent-cyan-600"
        />
      );
    },
    blockquote: ({ children }) => (
      <blockquote className="mb-2 border-l-2 pl-3 text-muted-foreground">{children}</blockquote>
    ),
    a: ({ href, children }) => (
      <a href={href} className="underline underline-offset-2" target="_blank" rel="noreferrer">
        {children}
      </a>
    ),
    code: ({ className, children }) => {
      const text = String(children ?? "").replace(/\n$/, "");
      const language = languageFromClassName(className);
      const isBlock = Boolean(className) || text.includes("\n");
      if (isBlock) {
        return <CodeBlock code={text} language={language} className="mb-0" />;
      }
      return (
        <code className="rounded bg-muted px-1 py-0.5 font-mono" style={{ fontSize: codeFontSize }}>
          {children}
        </code>
      );
    },
    pre: ({ children }) => {
      const items = Children.toArray(children);
      const only = items.length === 1 ? items[0] : null;
      if (isValidElement(only) && only.type === CodeBlock) {
        return <div className="mb-2 last:mb-0">{only as ReactNode}</div>;
      }
      return (
        <pre className="code-surface mb-2 overflow-auto rounded-md border border-border/70 px-3 py-2 last:mb-0">
          {children}
        </pre>
      );
    },
    table: ({ children }) => (
      <div className="mb-2 overflow-auto">
        <table className="w-full border-collapse text-sm">{children}</table>
      </div>
    ),
    th: ({ children }) => <th className="border-b px-2 py-1 text-left font-medium">{children}</th>,
    td: ({ children }) => <td className="border-b px-2 py-1 align-top">{children}</td>,
  };
}

export const AssistantMarkdown = memo(function AssistantMarkdown({
  text,
  variant = "default",
}: {
  text: string;
  variant?: "default" | "plan";
}) {
  const codeFontSize = useUiStore((state) => state.codeFontSize);
  const components = useMemo(
    () => markdownComponents(variant, codeFontSize),
    [codeFontSize, variant],
  );
  return (
    <div className="text-sm leading-6">
      <Markdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </Markdown>
    </div>
  );
});
