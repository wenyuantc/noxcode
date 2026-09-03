import { useEffect, useState } from "react";

import { onSshHostKeyChanged, onSshHostTrustRequest } from "@/lib/backend";
import type { SshHostKeyChanged, SshHostTrustPrompt } from "@/lib/types";

export function useSshTrustEvents() {
  const [prompt, setPrompt] = useState<SshHostTrustPrompt | null>(null);
  const [changed, setChanged] = useState<SshHostKeyChanged | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    void onSshHostTrustRequest((value) => setPrompt(value)).then((fn) => {
      if (cancelled) fn();
      else unlistens.push(fn);
    });
    void onSshHostKeyChanged((value) => setChanged(value)).then((fn) => {
      if (cancelled) fn();
      else unlistens.push(fn);
    });
    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
    };
  }, []);

  return { prompt, setPrompt, changed, setChanged };
}
