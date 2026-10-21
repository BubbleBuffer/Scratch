// The chat reference widgets: a chapter or section of a chat, shown in a note
// as a folding card (folded: the label, title and the decision or gist; open:
// the summary, decision, sections and Open in chat), and inline as a chip.
// These are thin adapters over the generic widgets in ui/; they know about
// ChatReference, the chat's colours and the chat's actions.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'chat_reference.dart';
import 'outline_names.dart';
import 'chat_reference_resolution.dart';

Color _edgeColour(BuildContext context, String name) {
  final c = context.colors;
  return spacePatch(name, c.dark ? Brightness.dark : Brightness.light)?.deep ??
      c.accent;
}

/// The chat reference card. A chapter keeps its stacked sheets behind the
/// face; a section does not. [onOpenInChat] null hides the action (a preview).
/// [open] and [onOpenChanged] let the caller own the open state.
class ChatReferenceCard extends StatelessWidget {
  const ChatReferenceCard({
    super.key,
    required this.reference,
    this.onOpenInChat,
    this.initiallyOpen = false,
    this.open,
    this.onOpenChanged,
    this.source,
    this.onCacheChanged,
  });

  final ChatReference reference;
  final VoidCallback? onOpenInChat;
  final bool initiallyOpen;
  final bool? open;
  final ValueChanged<bool>? onOpenChanged;
  final ChatReferenceSource? source;
  final ValueChanged<ChatReference>? onCacheChanged;

  @override
  Widget build(BuildContext context) => ResolvedChatReference(
    reference: reference,
    source: source,
    onCacheChanged: onCacheChanged,
    builder: (context, current, availability) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _card(context, current),
        if (availability == ChatReferenceAvailability.missing)
          _unavailable(context),
      ],
    ),
  );

  Widget _card(BuildContext context, ChatReference r) {
    final c = context.colors;
    final ty = context.type;
    final chapter = r.level == ChatRefLevel.chapter;
    final peekText = r.displayDecision ?? r.gist;
    final count = [
      if (chapter && r.when.isNotEmpty) r.when,
      if (chapter && r.sections.isNotEmpty) '${r.sections.length} sections',
    ].join(' · ');

    return FoldableStackCard(
      faceKey: const ValueKey('chatref-fold'),
      edge: _edgeColour(context, r.colour),
      label: r.label,
      title: r.title,
      titleMaxLines: 1,
      stacked: chapter,
      initiallyOpen: initiallyOpen,
      open: open,
      onOpenChanged: onOpenChanged,
      peek: peekText.isEmpty
          ? null
          : r.displayDecision != null
          ? DecisionLine(peekText, peek: true, color: c.textTertiary)
          : Text(
              peekText,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(ty.uiSmall, c.textTertiary),
            ),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (r.summary.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(r.summary, style: ts(ty.ui, c.textSecondary)),
            ),
          if (r.displayDecision != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: DecisionLine(r.displayDecision!, boxed: true),
            ),
          if (r.sections.isNotEmpty) ...[
            const SizedBox(height: 4),
            MarkedList(
              divided: true,
              rows: [
                for (final s in r.sections)
                  MarkedRow(
                    label: outlineNameGist(
                      s.title,
                      '',
                      chapterName: r.chapterName,
                    ).name,
                    colour: _edgeColour(context, s.colour),
                    small: true,
                    oneLine: true,
                    padding: const EdgeInsets.symmetric(vertical: 3),
                  ),
              ],
            ),
            const SizedBox(height: 8),
          ],
        ],
      ),
      footer: (isOpen) => !isOpen
          ? null
          : Row(
              children: [
                Expanded(
                  child: Text(count, style: ts(ty.uiSmall, c.textTertiary)),
                ),
                if (onOpenInChat != null)
                  FooterAction(
                    key: const ValueKey('chatref-open'),
                    label: 'Open in chat',
                    onTap: onOpenInChat,
                  ),
              ],
            ),
    );
  }
}

/// The inline reference: "● Chat › Chapter [› Section]". Tap (if [onTap]) shows
/// the summary; it is not a link in text.
class ChatReferenceChip extends StatelessWidget {
  const ChatReferenceChip({
    super.key,
    required this.reference,
    this.onTap,
    this.source,
    this.onCacheChanged,
  });
  final ChatReference reference;
  final VoidCallback? onTap;
  final ChatReferenceSource? source;
  final ValueChanged<ChatReference>? onCacheChanged;

  @override
  Widget build(BuildContext context) => ResolvedChatReference(
    reference: reference,
    source: source,
    onCacheChanged: onCacheChanged,
    builder: (context, current, availability) {
      final chip = ReferenceChip(
        segments: [
          current.chatTitle,
          current.chapterName,
          if (current.isSection) current.title,
        ],
        colour: _edgeColour(context, current.colour),
        onTap: onTap,
      );
      if (availability != ChatReferenceAvailability.missing) return chip;
      return Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [chip, _unavailable(context)],
      );
    },
  );
}

Widget _unavailable(BuildContext context) => Padding(
  padding: const EdgeInsets.only(top: 2, bottom: 4),
  child: Text(
    'chat unavailable',
    style: ts(context.type.uiSmall, context.colors.textTertiary),
  ),
);
