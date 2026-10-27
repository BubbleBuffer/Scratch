// The outline's chapter parts in the chat: a closed chapter's folding card,
// the heading of an open chapter and section dividers with their menu button.
// The chat lays them out (see agent_outline.dart) and decides what a tap does.
// The card is the same FoldableStackCard the note's chat references use; the
// rows are the generic ones in ui/.

import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_outline.dart';

/// The chapter's colour: a small patch, as spaces use. Chapters and sections
/// take colours in turn so neighbours differ.
Color outlineColor(BuildContext context, int index) =>
    spaceColor(
      spaceColorNames[index % spaceColorNames.length],
      context.colors.dark ? Brightness.dark : Brightness.light,
    ) ??
    context.colors.accent;

/// A closed chapter: one folding card, with the decision (or a gist line) on
/// its face. Opening it shows the summary, decision and sections; the foot
/// holds the count and the message actions. The card stays above its messages
/// when they are shown.
class OutlineChapterCard extends StatelessWidget {
  const OutlineChapterCard({
    super.key,
    required this.chapter,
    required this.color,
    required this.messagesShown,
    required this.onToggleMessages,
    required this.onAddToNote,
    required this.onOpenSection,
    this.open,
    this.onOpenChanged,
  });
  final ChapterView chapter;
  final Color color;
  final bool messagesShown;
  final VoidCallback onToggleMessages;
  final VoidCallback onAddToNote;
  final ValueChanged<SectionView> onOpenSection;
  final bool? open;
  final ValueChanged<bool>? onOpenChanged;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final decision = chapter.decision;
    final gist = chapter.gist;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: FoldableStackCard(
        faceKey: ValueKey('outline-card-face-${chapter.id}'),
        edge: color,
        label: _chapterLabel(chapter, closed: true),
        title: chapter.title,
        titleMaxLines: 1,
        stacked: true,
        open: open,
        onOpenChanged: onOpenChanged,
        peek: decision != null
            ? DecisionLine(decision, peek: true, color: c.textSecondary)
            : gist == null
            ? null
            : Text(
                gist,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                key: ValueKey('outline-card-gist-${chapter.id}'),
                style: ts(ty.uiSmall, c.textSecondary),
              ),
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (chapter.summary.isNotEmpty)
              Text(chapter.summary, style: ts(ty.body, c.textPrimary)),
            if (decision != null) ...[
              const SizedBox(height: 8),
              DecisionLine(decision),
            ],
            if (chapter.sections.isNotEmpty) ...[
              const SizedBox(height: 4),
              MarkedList(
                rows: [
                  for (final s in chapter.sections)
                    MarkedRow(
                      key: ValueKey('outline-card-section-${s.id}'),
                      onTap: () => onOpenSection(s),
                      label: s.title,
                      colour: outlineColor(
                        context,
                        chapter.sections.indexOf(s) + 1,
                      ),
                    ),
                ],
              ),
            ],
          ],
        ),
        footer: (open) => Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  outlineCountLine(chapter),
                  style: ts(ty.uiSmall, c.textTertiary),
                ),
              ),
              if (open) ...[
                FooterAction(
                  key: ValueKey('outline-card-add-${chapter.id}'),
                  label: 'Add to note',
                  onTap: onAddToNote,
                ),
                const SizedBox(width: 6),
              ],
              FooterAction(
                key: ValueKey('outline-card-show-${chapter.id}'),
                label: messagesShown ? 'Hide messages' : 'Show messages',
                onTap: onToggleMessages,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The heading of a chapter whose messages show: an open chapter, or a closed
/// one that was unfolded. [closed] adds a quiet "Chapter closed · Fold" action
/// that calls [onFold].
class OutlineChapterHeading extends StatelessWidget {
  const OutlineChapterHeading({
    super.key,
    required this.chapter,
    required this.color,
    required this.onMenu,
    this.closed = false,
    this.onFold,
  });
  final ChapterView chapter;
  final Color color;
  final VoidCallback onMenu;
  final bool closed;
  final VoidCallback? onFold;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (closed)
                Hover(
                  key: ValueKey('outline-chapter-fold-${chapter.id}'),
                  onTap: onFold,
                  child: Text(
                    'Chapter closed · Fold',
                    style: ts(context.type.uiSmall, c.textTertiary),
                  ),
                )
              else
                Text(
                  _chapterLabel(chapter, closed: false),
                  style: ts(context.type.uiSmall, c.textTertiary),
                ),
              Text(
                chapter.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: tsDisplay(context.type.h2, c.textPrimary),
              ),
            ],
          ),
        ),
        OutlineMoreButton(
          key: ValueKey('outline-chapter-menu-${chapter.id}'),
          onTap: onMenu,
        ),
      ],
    );
  }
}

/// A section's dashed divider in the transcript, with its menu.
class OutlineSectionDivider extends StatelessWidget {
  const OutlineSectionDivider({
    super.key,
    required this.section,
    required this.color,
    required this.onMenu,
  });
  final SectionView section;
  final Color color;
  final VoidCallback onMenu;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Row(
      children: [
        Container(
          width: 4,
          height: 13,
          decoration: BoxDecoration(
            color: color,
            borderRadius: BorderRadius.circular(2),
          ),
        ),
        const SizedBox(width: 8),
        Flexible(
          child: Text(
            section.title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: ts(context.type.uiSmall, c.textPrimary),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: CustomPaint(
            size: const Size(double.infinity, 1),
            painter: _DashedLinePainter(c.borderSubtle),
          ),
        ),
        OutlineMoreButton(
          key: ValueKey('outline-section-menu-${section.id}'),
          onTap: onMenu,
        ),
      ],
    );
  }
}

class OutlineMoreButton extends StatelessWidget {
  const OutlineMoreButton({super.key, required this.onTap});
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'More',
    child: AppTooltip(
      label: 'More',
      child: Hover(
        onTap: onTap,
        radius: Radii.md,
        padding: const EdgeInsets.all(4),
        child: Icon20(Ico.more, size: 16, color: context.colors.iconTertiary),
      ),
    ),
  );
}

class _DashedLinePainter extends CustomPainter {
  const _DashedLinePainter(this.color);
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = 1;
    var x = 0.0;
    while (x < size.width) {
      canvas.drawLine(Offset(x, 0.5), Offset(x + 4, 0.5), paint);
      x += 7;
    }
  }

  @override
  bool shouldRepaint(_DashedLinePainter old) => old.color != color;
}

String _sectionCount(ChapterView chapter) {
  final n = chapter.sections.length;
  return '$n ${n == 1 ? 'section' : 'sections'}';
}

String _messageCount(ChapterView chapter) {
  final n = chapter.messageCount;
  return '$n ${n == 1 ? 'message' : 'messages'}';
}

/// "12 messages · 3 sections" under a card and in the contents.
String outlineCountLine(ChapterView chapter) =>
    '${_messageCount(chapter)} · ${_sectionCount(chapter)}';

String _chapterLabel(ChapterView chapter, {required bool closed}) {
  final parts = [
    'Chapter',
    if (chapter.when.isNotEmpty) chapter.when,
    if (chapter.ongoing) 'ongoing' else if (closed) 'closed',
  ];
  return parts.join(' · ');
}
