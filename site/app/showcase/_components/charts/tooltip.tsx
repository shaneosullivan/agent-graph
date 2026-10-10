"use client";

import {useCallback, useState} from "react";

/** A tooltip that follows the pointer: `show` it on hover, `hide` it on leave, render `tip`. */
export function useTooltip() {
  const [state, setState] = useState<{
    x: number;
    y: number;
    content: React.ReactNode;
  } | null>(null);
  const show = useCallback((e: React.MouseEvent, content: React.ReactNode) => {
    setState({x: e.clientX, y: e.clientY, content});
  }, []);
  const hide = useCallback(() => setState(null), []);
  const tip = state ? (
    <div className="tooltip" style={{left: state.x + 14, top: state.y + 14}}>
      {state.content}
    </div>
  ) : null;
  return {tip, show, hide};
}
