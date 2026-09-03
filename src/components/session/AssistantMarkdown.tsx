import { memo } from "react";
import type { Components } from "react-markdown";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "@/lib/utils";

function markdownComponents(variant: "default" | "plan"): Components {
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
      const inline = !className;
      if (inline) {
        return (
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.8em]">{children}</code>
        );
      }
      return <code className={`block font-mono text-xs ${className ?? ""}`}>{children}</code>;
    },
    pre: ({ children }) => (
      <pre className="mb-2 overflow-auto rounded-md border bg-muted/40 px-3 py-2 text-xs leading-5 last:mb-0">
        {children}
      </pre>
    ),
    table: ({ children }) => (
      <div className="mb-2 overflow-auto">
        <table className="w-full border-collapse text-sm">{children}</table>
      </div>
    ),
    th: ({ children }) => <th className="border-b px-2 py-1 text-left font-medium">{children}</th>,
    td: ({ children }) => <td className="border-b px-2 py-1 align-top">{children}</td>,
  };
}

const defaultComponents = markdownComponents("default");
const planComponents = markdownComponents("plan");

export const AssistantMarkdown = memo(function AssistantMarkdown({
  text,
  variant = "default",
}: {
  text: string;
  variant?: "default" | "plan";
}) {
  return (
    <div className="text-sm leading-6">
      <Markdown
        remarkPlugins={[remarkGfm]}
        components={variant === "plan" ? planComponents : defaultComponents}
      >
        {text}
      </Markdown>
    </div>
  );
});
