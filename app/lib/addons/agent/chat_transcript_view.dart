// The rows of the chat: the plain list's spaced item and the widget of each
// kind of chat item. The outline's rows (cards, headings, dividers, flash and
// collapse) are the kit's OutlineList; it asks for an item's widget here.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_cells.dart';
import 'agent_markdown.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'agent_turn.dart';
import 'chat_reader_controller.dart';
import 'scratch_user_message.dart';

class ChatTranscriptView {
  const ChatTranscriptView({
    required this.context,
    required this.store,
    required this.reader,
  });

  final BuildContext context;
  final AgentStore store;
  final ChatReaderController reader;

  /// A row of the plain list: one chat item, spaced as the plain list always
  /// has been. Keyed by index, so the top row can be measured on screen.
  Widget plainRow(int i, ChatItem item) => Center(
    key: reader.plainRows.putIfAbsent(i, () => GlobalKey()),
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: Layout.contentWidth),
      child: Padding(
        padding: const EdgeInsets.only(bottom: 14),
        child: itemView(item),
      ),
    ),
  );

  Widget itemView(ChatItem item) => switch (item) {
    UserText() => ScratchUserMessage(
      message: item,
      sessionId: store.openId,
      onOpenChat: store.openSession,
      chatTitle: store.displaySessionTitle,
      key: ValueKey('agent-msg-${item.id}'),
    ),
    AgentTurn() => AgentTurnView(
      turn: item,
      store: store,
      key: ValueKey('agent-turn-${item.id}'),
    ),
    AssistantText() => AgentMarkdown(
      item.text,
      key: ValueKey('agent-msg-${item.id}'),
    ),
    CodeCell() => CodeCellRow(cell: item, key: ValueKey('agent-${item.id}')),
    RunNote() => Text(
      item.text,
      key: ValueKey('agent-note-${item.id}'),
      style: ts(
        context.type.uiSmall,
        item.error
            ? context.colors.inlineCodeText
            : context.colors.textTertiary,
      ),
    ),
  };
}
