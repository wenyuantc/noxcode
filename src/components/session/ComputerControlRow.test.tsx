import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import type { GroupedSessionItem } from "@/lib/sessionLines";
import { ComputerControlRow } from "./ComputerControlRow";

function createItem(
  id: string,
  text: string,
  extra?: Partial<GroupedSessionItem>,
): GroupedSessionItem {
  return {
    id,
    kind: "tool",
    text,
    createdAt: "2026-09-20T00:00:00Z",
    ...extra,
  };
}

describe("ComputerControlRow", () => {
  it("renders collapsed summary with app and action verbs when completed", () => {
    const items: GroupedSessionItem[] = [
      createItem("1", "[电脑控制] 截图", {
        tool: {
          phase: "result",
          call_id: "c1",
          name: "Computer",
          title: "电脑控制 截图",
          args_summary: "读取状态 Safari",
          ok: true,
          duration_ms: 200,
          image_names: ["win.png"],
        },
        images: [
          {
            name: "win.png",
            mime_type: "image/png",
            data_url: "data:image/png;base64,aaa",
          },
        ],
      }),
      createItem("2", "[电脑控制] 点击 (551, 100)", {
        tool: {
          phase: "result",
          call_id: "c2",
          name: "Computer",
          title: "电脑控制 点击 (551, 100)",
          args_summary: "点击 (551, 100)",
          ok: true,
          duration_ms: 50,
          image_names: [],
        },
      }),
      createItem("3", "[电脑控制] 等待 500 ms", {
        tool: {
          phase: "result",
          call_id: "c3",
          name: "Computer",
          title: "电脑控制 等待 500 ms",
          args_summary: "等待 500 ms",
          ok: true,
          duration_ms: 500,
          image_names: [],
        },
      }),
    ];

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ComputerControlRow items={items} running={false} />
      </I18nextProvider>,
    );

    // Collapsed by default when not running
    expect(html).toContain('aria-expanded="false"');
    // Shows app and step count with actions
    expect(html).toContain("Safari · 3 步 (截图 · 点击 · 等待)");
    expect(html).toContain("电脑控制");
  });

  it("automatically expands when running and shows current active step", () => {
    const items: GroupedSessionItem[] = [
      createItem("1", "[电脑控制] 点击 (551, 100)", {
        tool: {
          phase: "start",
          call_id: "c1",
          name: "Computer",
          title: "电脑控制 点击 (551, 100)",
          args_summary: "点击 (551, 100)",
          image_names: [],
        },
      }),
    ];

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ComputerControlRow items={items} running={true} />
      </I18nextProvider>,
    );

    // Expanded when running
    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain("正在执行");
    expect(html).toContain("电脑控制 · 正在 点击 (551, 100)");
    // Shows step details
    expect(html).toContain("点击 (551, 100)");
  });

  it("renders screenshot thumbnails and failed badge", () => {
    const items: GroupedSessionItem[] = [
      createItem("1", "[电脑控制] 截图", {
        tool: {
          phase: "result",
          call_id: "c1",
          name: "Computer",
          title: "电脑控制 截图",
          args_summary: "截图",
          ok: false,
          duration_ms: 320,
          image_names: ["screen.png"],
        },
        ok: false,
        images: [
          {
            name: "screen.png",
            mime_type: "image/png",
            data_url: "data:image/png;base64,xyz",
          },
        ],
      }),
    ];

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ComputerControlRow items={items} running={true} />
      </I18nextProvider>,
    );

    // Thumbnail is rendered
    expect(html).toContain('src="data:image/png;base64,xyz"');
    expect(html).toContain("320 ms");
    expect(html).toContain("失败");
  });
});
