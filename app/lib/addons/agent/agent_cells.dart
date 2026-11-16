
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
            padding: const EdgeInsets.symmetric(horizontal: Space.sm),
            child: SizedBox(
              height: 34,
              child: Row(
                children: [
                  Icon20(
                    _open ? Ico.chevronDown : Ico.chevronRight,
                    size: IconSize.xs,
                    color: c.iconTertiary,
                  ),
                  const Gap(Space.xs),
                  Icon20(Ico.code, size: IconSize.sm, color: c.iconSecondary),
                  const Gap(Space.sm),
                  Expanded(
                    child: Text(
                      cell.firstLine,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: ts(context.type.code, c.textSecondary, mono: true),
                    ),
                  ),
                  const Gap(Space.sm),
                  if (cell.status == CellStatus.running) ...[
                    Spinner(size: IconSize.xs, color: c.iconTertiary),
                    const Gap(Space.xs),
                  ],
                  Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Space.xs,
                      vertical: Space.hair,
                    ),
                    decoration: BoxDecoration(
                      color: bg,
                      borderRadius: Radii.pill,
                    ),
                    child: Text(
                      label,
                      style: ts(context.type.uiSmall, c.textPrimary, weight: Weight.semibold),
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (_open)
            Padding(
              padding: const EdgeInsets.fromLTRB(Space.sm, 0, Space.sm, Space.sm),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  CodeBox(
                    cell.code,
                    textKey: ValueKey('agent-${cell.id}-code'),
                  ),
                  if (cell.result != null) ...[
                    const Gap(Space.sm),
                    Text(
                      'Result',
                      style: ts(context.type.uiSmall, c.textTertiary),
                    ),
                    const Gap(Space.xxs),
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

