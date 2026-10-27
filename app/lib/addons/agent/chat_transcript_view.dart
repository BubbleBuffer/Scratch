// The rows of the chat: the plain list's spaced item, an outline row (a chat
// item, a chapter's card or heading, or a section divider) with its flash and
// collapse wrappers, and the widget of each kind of chat item. The chat decides
// what a tap on a row does and passes those actions in.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_cells.dart';
import 'agent_markdown.dart';
import 'agent_outline.dart';
import 'agent_outline_view.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'agent_turn.dart';
import 'chat_reader_controller.dart';
import 'outline_read_state.dart';
import 'scratch_user_message.dart';

class ChatTranscriptView {
  const ChatTranscriptView({
    required this.context,
    required this.store,
    required this.reader,
    required this.read,
    required this.onChapterMenu,
    required this.onSectionMenu,
    required this.onAddToNote,
    required this.onExpandedChanged,
  });

  final BuildContext context;
  final AgentStore store;
  final ChatReaderController reader;
  final OutlineReadState read;
  final void Function(ChapterView chapter) onChapterMenu;
  final void Function(ChapterView chapter, SectionView section) onSectionMenu;
  final void Function(ChapterView chapter, SectionView? section) onAddToNote;
  final void Function(String first, bool open) onExpandedChanged;

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

  /// The row widget for [row]: the chat item, a chapter's card or heading, or
  /// a section divider.
  Widget rowView(OutlineRow row) {
    final outline = reader.outline;
    final Widget child = switch (row) {
      ItemRow(:final item) => itemView(item),
      ChapterRow(:final folded) => _chapterRow(
        outline.chapters[row.chapter],
        row.chapter,
        folded,
      ),
      SectionRow() => OutlineSectionDivider(
        key: ValueKey('outline-section-${row.key}'),
        section: outline.chapters[row.chapter].sections[row.section],
        color: outlineColor(context, row.section + 1),
        onMenu: () => onSectionMenu(
          outline.chapters[row.chapter],
          outline.chapters[row.chapter].sections[row.section],
        ),
      ),
    };
    final view = Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: Layout.contentWidth),
        child: Padding(
          padding: const EdgeInsets.only(bottom: 14),
          child: reader.flashKeys.contains(row.key)
              ? OutlineFlash(token: reader.flashToken, child: child)
              : child,
        ),
      ),
    );
    if (row is! ItemRow) return view;
    // A collapsing chapter's messages shrink to nothing. Every item row keeps
    // this wrapper, at full size when not collapsing, so the row's subtree does
    // not change shape when a collapse starts.
    final chapter = row.chapter >= 0 && row.chapter < outline.chapters.length
        ? outline.chapters[row.chapter].first
        : null;
    final Animation<double> collapse =
        (chapter == null ? null : read.collapsing[chapter]) ??
        OutlineReadState.full;
    return SizeTransition(
      sizeFactor: collapse,
      alignment: Alignment.topCenter,
      child: view,
    );
  }

  Widget _chapterRow(ChapterView chapter, int index, bool folded) {
    final color = outlineColor(context, index);
    if (chapter.ongoing) {
      return OutlineChapterHeading(
        key: ValueKey('outline-chapter-${chapter.id}'),
        chapter: chapter,
        color: color,
        onMenu: () => onChapterMenu(chapter),
        closed: false,
        onFold: null,
      );
    }
    if (read.deferred.contains(chapter.first)) {
      return OutlineChapterHeading(
        key: ValueKey('outline-chapter-${chapter.id}'),
        chapter: chapter,
        color: color,
        onMenu: () => onChapterMenu(chapter),
        closed: true,
        onFold: () => read.foldNow(chapter.first),
      );
    }
    final shown = !folded && !read.collapsing.containsKey(chapter.first);
    return OutlineChapterCard(
      key: ValueKey('outline-chapter-${chapter.id}'),
      chapter: chapter,
      color: color,
      messagesShown: shown,
      onToggleMessages: () => reader.toggleMessages(chapter, !shown),
      onAddToNote: () => onAddToNote(chapter, null),
      onOpenSection: (section) => reader.jump(
        section.first,
        section.last,
        sectionKey: 'sec:${section.id}',
      ),
      open: read.expanded[chapter.first] ?? false,
      onOpenChanged: (open) => onExpandedChanged(chapter.first, open),
    );
  }

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
    ApprovalItem() => ApprovalCard(
      approval: item.approval,
      key: ValueKey('agent-approval-${item.approval.id}'),
      onDecide: (decision, reason) =>
          store.decide(item.approval.id, decision, reason: reason),
    ),
  };
}
