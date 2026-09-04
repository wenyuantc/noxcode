import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./select";

describe("Select label resolution", () => {
  it("renders item label/name instead of raw key/value in SelectValue", () => {
    const html = renderToString(
      <Select value="yolo">
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="default">变更前确认</SelectItem>
          <SelectItem value="edit">自动编辑</SelectItem>
          <SelectItem value="build">自动构建</SelectItem>
          <SelectItem value="yolo">完全访问</SelectItem>
        </SelectContent>
      </Select>,
    );

    expect(html).toContain("完全访问");
    expect(html).not.toContain("<span>yolo</span>");
  });

  it("resolves mapped subagent policy names correctly", () => {
    const policies = [
      { id: "conservative", name: "保守" },
      { id: "balanced", name: "均衡" },
      { id: "aggressive", name: "激进" },
    ];

    const html = renderToString(
      <Select value="balanced">
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {policies.map((p) => (
            <SelectItem key={p.id} value={p.id}>
              {p.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>,
    );

    expect(html).toContain("均衡");
    expect(html).not.toContain("<span>balanced</span>");
  });
});
