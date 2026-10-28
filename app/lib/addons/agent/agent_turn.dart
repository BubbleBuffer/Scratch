import 'navigation.dart';

import 'package:agent_kit_core/agent_kit_core.dart' as kit
    show ActivityGroup, ToolRow, WorkState;
import 'package:agent_kit/agent_kit.dart' as kit show ActivityGroupView;
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/ui.dart';
import 'agent_cells.dart';
import 'agent_markdown.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'kit_backend.dart' show kitToolKind;

class AgentTurnView extends StatelessWidget {
  const AgentTurnView({super.key, required this.turn, required this.store});
  final AgentTurn turn;
  final AgentStore store;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final text in turn.texts)
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: AgentMarkdown(
              text.text,
              key: ValueKey('agent-msg-${text.id}'),
            ),
          ),
        if (turn.streaming.trim().isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: AgentMarkdown(
              turn.streaming,
              key: const ValueKey('agent-msg-live'),
            ),
          ),
        for (final edit in turn.edits)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: AgentEditRow(
              key: ValueKey('agent-edit-${edit.id}'),
              edit: edit,
              sessionId: store.openId,
              onUndo: () => store.undoEdit(edit.id),
            ),
          ),
        if (turn.activity.isNotEmpty || turn.cells.isNotEmpty)
          AgentWorkRow(key: ValueKey('agent-work-${turn.id}'), turn: turn),
        if (turn.live)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Row(
              children: [
                Spinner(size: 13, color: c.iconTertiary),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    'Working${(store.latestActivitySummary ?? turn.activity.lastOrNull?.summary ?? '').isEmpty ? '' : ' · ${store.latestActivitySummary ?? turn.activity.last.summary}'}',
                    key: const ValueKey('agent-working'),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: ts(context.type.uiSmall, c.textTertiary),
                  ),
                ),
              ],
            ),
          ),
        if (turn.run?.status == 'cancelled')
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              'Stopped.',
              style: ts(context.type.uiSmall, c.textTertiary),
            ),
          ),
        if ((turn.run?.error ?? '').isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              turn.run!.error!,
              style: ts(context.type.uiSmall, c.inlineCodeText),
            ),
          ),
      ],
    );
  }
}

/// Tool names, rather than their summaries, determine stable work counts.
String workSummary(AgentTurn turn) {
  final names = turn.activity.isEmpty
      ? turn.cells.map((c) => c.tool)
      : turn.activity.map((a) => a.name);
  final counts = <String, int>{};
  for (final name in names) {
    final group = switch (name) {
      'search_notes' ||
      'search' ||
      'search_pages' ||
      'search_blocks' ||
      'vault_search' => 'searched',
      'get_page' ||
      'read_page' ||
      'page_get' ||
      'read_file' ||
      'get_blocks' ||
      'query_database' ||
      'agenda' => 'read',
      'read' => 'read_files',
      'write' || 'edit' => 'edited_files',
      'list' => 'listed',
      'create_page' ||
      'edit_page' ||
      'upsert_row' ||
      'replace_page' ||
      'update_block' ||
      'delete_block' ||
      'move_blocks' ||
      'move_page' ||
      'trash_page' ||
      'delete_page' ||
      'rename_page' ||
      'update_task' ||
      'daily_note' => 'edited',
      'open_page' ||
      'highlight' ||
      'highlight_blocks' ||
      'ui_open_page' ||
      'ui_highlight' => 'opened',
      'exec' || 'shell' => 'ran',
      _ => 'other',
    };
    counts[group] = (counts[group] ?? 0) + 1;
  }
  final failed = turn.activity.isEmpty
      ? turn.cells.where((c) => c.status == CellStatus.failed).length
      : turn.activity.where((a) => a.isError).length;
  return [
    for (final label in [
      'searched',
      'read',
      'read_files',
      'edited',
      'edited_files',
      'listed',
      'opened',
      'ran',
      'other',
    ])
      if (counts.containsKey(label))
        switch (label) {
          'searched' => 'Searched ${counts[label]}×',
          'read' =>
            'Read ${counts[label]} ${counts[label] == 1 ? 'page' : 'pages'}',
          'edited' =>
            'Edited ${counts[label]} ${counts[label] == 1 ? 'page' : 'pages'}',
          'read_files' =>
            'Read ${counts[label]} ${counts[label] == 1 ? 'file' : 'files'}',
          'edited_files' =>
            'Edited ${counts[label]} ${counts[label] == 1 ? 'file' : 'files'}',
          'listed' => 'Listed ${counts[label]}×',
          'opened' =>
            'Opened ${counts[label]} ${counts[label] == 1 ? 'page' : 'pages'}',
          'ran' =>
            'Ran ${counts[label]} ${counts[label] == 1 ? 'command' : 'commands'}',
          _ =>
            '${counts[label]} ${counts[label] == 1 ? 'other call' : 'other calls'}',
        },
    if (failed > 0) '$failed failed',
  ].join(' · ');
}

/// A turn's tool work: the kit's activity group under Scratch's verb line,
/// with "Show code" for the turn's code cells once the group is open.
class AgentWorkRow extends StatelessWidget {
  const AgentWorkRow({super.key, required this.turn});
  final AgentTurn turn;

  kit.ActivityGroup _group() => kit.ActivityGroup(
    id: turn.id,
    rows: turn.activity.isEmpty
        ? [
            for (final cell in turn.cells)
              kit.ToolRow(
                id: cell.id,
                name: cell.tool,
                kind: kitToolKind(cell.tool),
                target: cell.firstLine,
                state: switch (cell.status) {
                  CellStatus.failed => kit.WorkState.failed,
                  _ => kit.WorkState.done,
                },
              ),
          ]
        : [
            for (final a in turn.activity)
              kit.ToolRow(
                id: '${a.callId}-${a.order}',
                name: a.name,
                kind: kitToolKind(a.name),
                target: a.summary,
                state: a.isError ? kit.WorkState.failed : kit.WorkState.done,
              ),
          ],
  );

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(left: 6),
    child: kit.ActivityGroupView(
      _group(),
      initiallyOpen: false,
      summaryOf: (_) => workSummary(turn),
      footer: turn.cells.isNotEmpty && !turn.live
          ? _CodeDisclosure(turn: turn)
          : null,
    ),
  );
}

class _CodeDisclosure extends StatefulWidget {
  const _CodeDisclosure({required this.turn});
  final AgentTurn turn;
  @override
  State<_CodeDisclosure> createState() => _CodeDisclosureState();
}

class _CodeDisclosureState extends State<_CodeDisclosure> {
  bool _code = false;
  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Align(
        alignment: Alignment.centerLeft,
        child: TextBtn(
          _code ? 'Hide code' : 'Show code',
          key: ValueKey('agent-work-${widget.turn.id}-code'),
          height: 28,
          onTap: () => setState(() => _code = !_code),
        ),
      ),
      if (_code)
        for (final cell in widget.turn.cells)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: CodeCellRow(
              key: ValueKey('agent-${cell.id}'),
              cell: cell,
            ),
          ),
    ],
  );
}

class AgentEditRow extends StatefulWidget {
  const AgentEditRow({
    super.key,
    required this.edit,
    required this.onUndo,
    this.sessionId,
  });
  final AgentEdit edit;
  final Future<AgentEdit> Function() onUndo;
  final String? sessionId;
  @override
  State<AgentEditRow> createState() => _AgentEditRowState();
}

class _AgentEditRowState extends State<AgentEditRow> {
  bool _open = false, _all = false, _undoing = false;
  AgentEdit? _result;
  String? _refusal;
  @override
  void didUpdateWidget(AgentEditRow oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.edit != widget.edit) _result = null;
  }

  Future<void> _undo() async {
    setState(() {
      _undoing = true;
      _refusal = null;
    });
    try {
      final result = await widget.onUndo();
      if (mounted) setState(() => _result = result);
    } catch (e) {
      if (mounted) {
        setState(
          () => _refusal = '$e'.replaceFirst(RegExp(r'^Exception: '), ''),
        );
      }
    } finally {
      if (mounted) setState(() => _undoing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final edit = _result ?? widget.edit;
    final pages = context.watch<PagesStore>();
    final pageName = displayTitle(
      edit.pageTitle,
      firstLine: pages.meta(edit.pageId)?.firstLine ?? '',
    );
    final c = context.colors;
    final green = c.dark ? const Color(0xFF93BD9B) : const Color(0xFF397748);
    final red = c.dark ? const Color(0xFFE0A3A3) : const Color(0xFFAD4B4B);
    final label = switch (edit.kind) {
      'created' => 'Created $pageName',
      'trashed' => 'Moved $pageName to trash',
      'row' => 'Updated a row in $pageName',
      _ => 'Edited $pageName',
    };
    final lines = [for (final h in edit.hunks) ...h.lines];
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: c.borderSubtle),
        borderRadius: Radii.field,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
            child: LayoutBuilder(
              builder: (context, bounds) => Wrap(
                crossAxisAlignment: WrapCrossAlignment.center,
                spacing: 4,
                children: [
                  Hover(
                    key: ValueKey('agent-edit-${edit.id}-toggle'),
                    onTap: lines.isEmpty
                        ? null
                        : () => setState(() => _open = !_open),
                    padding: const EdgeInsets.symmetric(
                      horizontal: 2,
                      vertical: 4,
                    ),
                    radius: Radii.sm,
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon20(
                          _open ? Ico.chevronDown : Ico.chevronRight,
                          size: 12,
                          color: c.iconTertiary,
                        ),
                        const SizedBox(width: 4),
                        Icon20(
                          edit.kind == 'trashed'
                              ? Ico.trash
                              : edit.kind == 'created'
                              ? Ico.plus
                              : Ico.compose,
                          size: 14,
                          color: c.iconSecondary,
                        ),
                        const SizedBox(width: 5),
                        ConstrainedBox(
                          constraints: BoxConstraints(
                            maxWidth: (bounds.maxWidth - 39)
                                .clamp(0.0, 250.0)
                                .toDouble(),
                          ),
                          child: Text(
                            label,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                            style: ts(
                              context.type.uiSmall,
                              edit.undone ? c.textTertiary : c.textPrimary,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  if (edit.kind != 'created' && edit.kind != 'trashed') ...[
                    Text(
                      '+${edit.added}',
                      style: ts(context.type.uiSmall, green),
                    ),
                    Text(
                      '−${edit.removed}',
                      style: ts(context.type.uiSmall, red),
                    ),
                  ],
                  IntrinsicWidth(
                    child: TextBtn(
                      'Open',
                      key: ValueKey('agent-edit-${edit.id}-open'),
                      height: 28,
                      onTap: () => openAgentPage(
                        context,
                        edit.pageId,
                        edit.pageTitle,
                        blocks: edit.changedBlocks,
                        sessionId: widget.sessionId,
                      ),
                    ),
                  ),
                  if (edit.undone || edit.undoable)
                    IntrinsicWidth(
                      child: TextBtn(
                        edit.undone
                            ? 'Undone'
                            : _undoing
                            ? 'Undoing…'
                            : 'Undo',
                        key: ValueKey('agent-edit-${edit.id}-undo'),
                        height: 28,
                        color: edit.undone ? c.textTertiary : null,
                        onTap: edit.undone || _undoing ? null : _undo,
                      ),
                    ),
                ],
              ),
            ),
          ),
          if (_refusal != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
              child: Text(
                _refusal!,
                key: ValueKey('agent-edit-${edit.id}-refusal'),
                style: ts(context.type.uiSmall, c.textTertiary),
              ),
            ),
          if (_open)
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 2, 10, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (i, line)
                      in (_all ? lines : lines.take(12)).indexed)
                    Container(
                      color: line.tag == '+'
                          ? c.okBg
                          : line.tag == '-'
                          ? c.dangerBg
                          : null,
                      padding: const EdgeInsets.symmetric(
                        horizontal: 5,
                        vertical: 1,
                      ),
                      child: Text(
                        '${line.tag == '=' ? ' ' : line.tag} ${line.text}',
                        key: ValueKey('agent-edit-${edit.id}-line-$i'),
                        style:
                            ts(
                              context.type.code,
                              line.tag == '+'
                                  ? green
                                  : line.tag == '-'
                                  ? red
                                  : c.textSecondary,
                              mono: true,
                            ).copyWith(
                              decoration: line.tag == '-'
                                  ? TextDecoration.lineThrough
                                  : null,
                            ),
                      ),
                    ),
                  if (!_all && lines.length > 12)
                    Align(
                      alignment: Alignment.centerLeft,
                      child: TextBtn(
                        'Show all',
                        key: ValueKey('agent-edit-${edit.id}-all'),
                        height: 28,
                        onTap: () => setState(() => _all = true),
                      ),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}
