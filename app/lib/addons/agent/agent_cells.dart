
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_transcript.dart';
import 'agent_markdown.dart';

class CodeCellRow extends StatefulWidget {
  const CodeCellRow({super.key, required this.cell});
  final CodeCell cell;

  @override
  State<CodeCellRow> createState() => _CodeCellRowState();
}

class _CodeCellRowState extends State<CodeCellRow> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final cell = widget.cell;
    final (label, bg) = switch (cell.status) {
      CellStatus.running => ('Running', c.infoBg),
      CellStatus.done => ('Done', c.okBg),
      CellStatus.failed => ('Failed', c.dangerBg),
    };
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: c.border),
        borderRadius: Radii.field,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Hover(
            key: ValueKey('agent-${cell.id}-toggle'),
            onTap: () => setState(() => _open = !_open),
            borderRadius: Radii.field,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: SizedBox(
              height: 34,
              child: Row(
                children: [
                  Icon20(
                    _open ? Ico.chevronDown : Ico.chevronRight,
                    size: 14,
                    color: c.iconTertiary,
                  ),
                  const SizedBox(width: 6),
                  Icon20(Ico.code, size: 16, color: c.iconSecondary),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      cell.firstLine,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: ts(context.type.code, c.textSecondary, mono: true),
                    ),
                  ),
                  const SizedBox(width: 8),
                  if (cell.status == CellStatus.running) ...[
                    Spinner(size: 12, color: c.iconTertiary),
                    const SizedBox(width: 6),
                  ],
                  Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 6,
                      vertical: 1,
                    ),
                    decoration: BoxDecoration(
                      color: bg,
                      borderRadius: Radii.pill,
                    ),
                    child: Text(
                      label,
                      style: tsRaw(12, 18, 600, c.textPrimary),
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (_open)
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  CodeBox(
                    cell.code,
                    textKey: ValueKey('agent-${cell.id}-code'),
                  ),
                  if (cell.result != null) ...[
                    const SizedBox(height: 8),
                    Text(
                      'Result',
                      style: ts(context.type.uiSmall, c.textTertiary),
                    ),
                    const SizedBox(height: 4),
                    CodeBox(
                      cell.result!,
                      textKey: ValueKey('agent-${cell.id}-result'),
                      color: cell.status == CellStatus.failed
                          ? c.inlineCodeText
                          : null,
                    ),
                  ],
                ],
              ),
            ),
        ],
      ),
    );
  }
}

