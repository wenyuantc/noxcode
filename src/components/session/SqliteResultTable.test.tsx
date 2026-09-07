import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { parseSqliteResult } from "@/lib/sessionLines";
import { SqliteResultTable } from "./SqliteResultTable";

describe("SqliteResultTable", () => {
  it("parses valid sqlite json result", () => {
    const raw = JSON.stringify({
      columns: ["id", "name", "score"],
      rows: [
        [1, "alice", 95.5],
        [2, "bob", null],
      ],
      row_count: 2,
      truncated: false,
    });
    const parsed = parseSqliteResult(raw);
    expect(parsed).toEqual({
      columns: ["id", "name", "score"],
      rows: [
        [1, "alice", 95.5],
        [2, "bob", null],
      ],
      rowCount: 2,
      truncated: false,
    });
  });

  it("returns null for non-sqlite or invalid json", () => {
    expect(parseSqliteResult("not json")).toBeNull();
    expect(parseSqliteResult(JSON.stringify({ not: "sqlite" }))).toBeNull();
  });

  it("renders table with columns, rows and row count", () => {
    const data = {
      columns: ["id", "title"],
      rows: [
        [1, "first"],
        [2, "second"],
      ],
      rowCount: 2,
      truncated: false,
    };
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <SqliteResultTable data={data} rawResult={JSON.stringify(data)} />
      </I18nextProvider>,
    );

    expect(html).toContain("2 行");
    expect(html).toContain("id");
    expect(html).toContain("title");
    expect(html).toContain("first");
    expect(html).toContain("second");
    expect(html).not.toContain("结果已截断");
  });

  it("renders truncation indicator when truncated is true", () => {
    const data = {
      columns: ["id"],
      rows: [[1]],
      rowCount: 100,
      truncated: true,
    };
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <SqliteResultTable data={data} rawResult={JSON.stringify(data)} />
      </I18nextProvider>,
    );

    expect(html).toContain("结果已截断");
  });

  it("renders empty state when rows are empty", () => {
    const data = {
      columns: ["id"],
      rows: [],
      rowCount: 0,
      truncated: false,
    };
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <SqliteResultTable data={data} rawResult={JSON.stringify(data)} />
      </I18nextProvider>,
    );

    expect(html).toContain("无数据");
  });
});
