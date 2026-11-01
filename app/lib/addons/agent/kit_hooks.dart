// Scratch's look for the kit pane's transcript: its markdown, the user
// message with chips that read live page titles, the edit row's wording and
// the verb summary, with "Show code" under an open activity group. Each hook
// replaces the kit's default for one kind of item; the rest stays the kit's.

import 'package:agent_kit/agent_kit.dart';
import 'package:agent_kit_core/agent_kit_core.dart' as kit;
import 'package:flutter/widgets.dart';

import 'agent_markdown.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'agent_turn.dart';
import 'kit_backend.dart' show kitContext;
import 'scratch_user_message.dart';

/// The hooks for [AgentChatPane.transcriptActions]. [context] belongs to the
/// screen that hosts the pane (page links open through it).
TranscriptActions scratchTranscriptActions(
  BuildContext context,
  AgentStore store,
) => TranscriptActions(
  markdown: const ScratchMarkdownRenderer(),
  summaryOf: kitVerbSummary,
  activityFooter: kitCodeFooter,
  itemBuilder: (_, item, kitWidget) => switch (item) {
    final kit.UserMessage m => ScratchUserMessage(
      key: ValueKey('agent-msg-${m.id}'),
      message: UserText(m.id, m.text, context: kitContext(m.refs)),
      sessionId: store.openId,
      onOpenChat: store.openSession,
      chatTitle: store.displaySessionTitle,
    ),
    final kit.FileEdit e => switch (_editOf(store, e)) {
      final edit? => AgentEditRow(
        key: ValueKey('agent-edit-row-${edit.id}'),
        edit: edit,
        sessionId: store.openId,
        onUndo: () => store.undoEdit(edit.id),
      ),
      null => kitWidget,
    },
    _ => kitWidget,
  },
);

AgentEdit? _editOf(AgentStore store, kit.FileEdit e) {
  const prefix = 'edit:';
  if (!e.id.startsWith(prefix)) return null;
  final id = e.id.substring(prefix.length);
  return store.state?.edits.where((x) => x.id == id).firstOrNull;
}

/// The activity group's verb line, counted from its tool rows.
String kitVerbSummary(kit.ActivityGroup g) => verbSummary(
  [for (final r in g.rows.whereType<kit.ToolRow>()) r.name],
  g.rows
      .whereType<kit.ToolRow>()
      .where((r) => r.state == kit.WorkState.failed)
      .length,
);

/// "Show code" for a finished group's tool calls.
Widget? kitCodeFooter(kit.ActivityGroup g) {
  final rows = g.rows.whereType<kit.ToolRow>().toList();
  if (rows.isEmpty || rows.any((r) => r.state == kit.WorkState.running)) {
    return null;
  }
  return CodeDisclosure(
    id: g.id,
    cells: [
      for (final r in rows)
        CodeCell(
          'cell-${r.id}',
          tool: r.name,
          code: codeOfArguments(r.arguments ?? ''),
          result: r.output == null ? null : cellResult(r.output!.text),
          status: switch (r.state) {
            kit.WorkState.running => CellStatus.running,
            kit.WorkState.failed ||
            kit.WorkState.cancelled => CellStatus.failed,
            _ => CellStatus.done,
          },
        ),
    ],
  );
}
