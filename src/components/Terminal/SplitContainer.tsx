import { useState, useCallback, useRef } from 'react';
import { TerminalPane } from './TerminalPane';
import type { SplitNode } from '../../types';

interface SplitContainerProps {
  node: SplitNode;
  isActive: boolean;
  activeTerminalId: string | null;
  onSplit: (terminalId: string, direction: 'horizontal' | 'vertical') => void;
  onClose: (terminalId: string) => void;
  onFocusTerminal: (terminalId: string) => void;
  onOpenRecording?: (filePath: string) => void;
}

export function SplitContainer({ node, isActive, activeTerminalId, onSplit, onClose, onFocusTerminal, onOpenRecording }: SplitContainerProps) {
  if (node.type === 'terminal') {
    return (
      <TerminalPane
        key={node.terminalId}
        terminalId={node.terminalId}
        isActive={isActive}
        onSplit={(direction) => onSplit(node.terminalId, direction)}
        onClosePane={() => onClose(node.terminalId)}
        onFocus={() => onFocusTerminal(node.terminalId)}
        onOpenRecording={onOpenRecording}
      />
    );
  }

  return (
    <SplitNodeView
      node={node}
      isActive={isActive}
      activeTerminalId={activeTerminalId}
      onSplit={onSplit}
      onClose={onClose}
      onFocusTerminal={onFocusTerminal}
      onOpenRecording={onOpenRecording}
    />
  );
}

/** Inner component for split nodes so we can use hooks unconditionally. */
function SplitNodeView({ node, isActive, activeTerminalId, onSplit, onClose, onFocusTerminal, onOpenRecording }: SplitContainerProps & { node: Extract<SplitNode, { type: 'split' }> }) {
  const [ratio, setRatio] = useState(node.ratio);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  // 水平分屏 = 上下排列(column), 垂直分屏 = 左右排列(row)
  const isHorizontal = node.direction === 'vertical';

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;
    const startPos = isHorizontal ? e.clientX : e.clientY;
    const startRatio = ratio;

    const onMove = (ev: MouseEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const totalSize = isHorizontal ? rect.width : rect.height;
      if (totalSize < 10) return;
      const currentPos = isHorizontal ? ev.clientX : ev.clientY;
      const delta = currentPos - startPos;
      const newRatio = Math.max(0.1, Math.min(0.9, startRatio + delta / totalSize));
      setRatio(newRatio);
    };

    const onUp = () => {
      dragging.current = false;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }, [isHorizontal, ratio]);

  // Account for the 4px divider by using calc()
  const firstSize = `calc(${ratio * 100}% - 2px)`;
  const secondSize = `calc(${(1 - ratio) * 100}% - 2px)`;

  return (
    <div
      ref={containerRef}
      style={{
        display: 'flex',
        flexDirection: isHorizontal ? 'row' : 'column',
        width: '100%',
        height: '100%',
      }}
    >
      <div style={{
        [isHorizontal ? 'width' : 'height']: firstSize,
        position: 'relative',
        overflow: 'hidden',
        flexShrink: 0,
      }}>
        <SplitContainer
          node={node.first}
          isActive={isActive}
          activeTerminalId={activeTerminalId}
          onSplit={onSplit}
          onClose={onClose}
          onFocusTerminal={onFocusTerminal}
          onOpenRecording={onOpenRecording}
        />
      </div>
      {/* Draggable divider */}
      <div
        style={{
          [isHorizontal ? 'width' : 'height']: '4px',
          cursor: isHorizontal ? 'col-resize' : 'row-resize',
          background: '#21262d',
          flexShrink: 0,
        }}
        onMouseDown={handleMouseDown}
      />
      <div style={{
        [isHorizontal ? 'width' : 'height']: secondSize,
        position: 'relative',
        overflow: 'hidden',
        flexShrink: 0,
      }}>
        <SplitContainer
          node={node.second}
          isActive={isActive}
          activeTerminalId={activeTerminalId}
          onSplit={onSplit}
          onClose={onClose}
          onFocusTerminal={onFocusTerminal}
          onOpenRecording={onOpenRecording}
        />
      </div>
    </div>
  );
}
