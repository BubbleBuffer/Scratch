// The outline's parts in the chat: a closed chapter's folding card, the heading
// of an open chapter, section dividers with their menu, the where-am-I strip
// over the transcript and the contents sheet. The chat lays them out (see
// agent_outline.dart) and decides what a tap does. The card is the same
// FoldableStackCard the note's chat references use; the rows and pickers are
// the generic ones in ui/.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'agent_outline.dart';

/// Where the reader is: the chapter and section titles and ids at the top of
/// the transcript, or nulls above the first chapter.
typedef OutlineWhere = ({
  String? chapter,
  String? section,
  String? chapterId,
  String? sectionId,
});

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
                  _countLine(chapter),
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

/// Where the reader is: the chapter and section at the top of the transcript.
/// Tapping it opens the contents.
class OutlineWhereStrip extends StatelessWidget {
  const OutlineWhereStrip({
    super.key,
    required this.where,
    required this.onTap,
    this.colors = const {},
  });

  final Map<String, Color> colors;

  /// The chapter and section titles, or nulls above the first chapter.
  final ValueListenable<OutlineWhere> where;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return ValueListenableBuilder<OutlineWhere>(
      valueListenable: where,
      builder: (context, w, _) => DecoratedBox(
        decoration: BoxDecoration(
          color: c.bgSecondary,
          border: Border(bottom: BorderSide(color: c.border)),
        ),
        child: Hover(
          key: const ValueKey('outline-where'),
          onTap: onTap,
          radius: 0,
          padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
          child: SizedBox(
            height: 40,
            child: Row(
              children: [
                Container(
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: colors[w.chapterId] ?? c.accent,
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text.rich(
                    TextSpan(
                      children: [
                        TextSpan(
                          text: w.chapter ?? 'Chat',
                          style: ts(
                            context.type.ui,
                            c.textPrimary,
                          ).copyWith(fontWeight: FontWeight.w600),
                        ),
                        if (w.section != null)
                          TextSpan(
                            text: '  ›  ${w.section}',
                            style: ts(context.type.ui, c.textTertiary),
                          ),
                      ],
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                const SizedBox(width: 12),
                Text('Contents ▾', style: ts(context.type.ui, c.textTertiary)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Briefly tints a row after a jump lands on it: a quarter-second hold, then
/// a fade over the rest of [OutlineFlash.duration]. A new [token] restarts it.
class OutlineFlash extends StatefulWidget {
  const OutlineFlash({super.key, required this.token, required this.child});
  final int token;
  final Widget child;

  static const duration = Duration(milliseconds: 1600);

  @override
  State<OutlineFlash> createState() => _OutlineFlashState();
}

class _OutlineFlashState extends State<OutlineFlash>
    with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: OutlineFlash.duration,
  );

  @override
  void initState() {
    super.initState();
    // Mounted by a jump: the highlight starts now, not on the next token.
    if (widget.token != 0) _c.forward(from: 0);
  }

  @override
  void didUpdateWidget(OutlineFlash old) {
    super.didUpdateWidget(old);
    if (old.token != widget.token && widget.token != 0) {
      _c.forward(from: 0);
    }
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final tint = context.colors.accent;
    return AnimatedBuilder(
      animation: _c,
      child: widget.child,
      builder: (context, child) {
        // Fully tinted for the first 40 %, then fading out.
        final t = _c.value;
        final alpha = t >= 1 ? 0.0 : (t < 0.4 ? 0.22 : 0.22 * (1 - t) / 0.6);
        return DecoratedBox(
          decoration: BoxDecoration(
            color: tint.withValues(alpha: alpha),
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          child: child,
        );
      },
    );
  }
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
String _countLine(ChapterView chapter) =>
    '${_messageCount(chapter)} · ${_sectionCount(chapter)}';

String _chapterLabel(ChapterView chapter, {required bool closed}) {
  final parts = [
    'Chapter',
    if (chapter.when.isNotEmpty) chapter.when,
    if (chapter.ongoing) 'ongoing' else if (closed) 'closed',
  ];
  return parts.join(' · ');
}

/// The contents: every chapter with its decision and sections, and the place
/// the reader is. On a phone it is a bottom sheet (70 % of the screen); on
/// desktop a centred panel. A tap closes the contents and calls [onChapter] or
/// [onSection].
Future<void> showOutlineContents(
  BuildContext context, {
  required OutlineView outline,
  required OutlineWhere where,
  required ValueChanged<ChapterView> onChapter,
  required void Function(ChapterView chapter, SectionView section) onSection,
}) => showPickerSheet<void>(
  context,
  (ctx) => _contents(
    ctx,
    outline: outline,
    where: where,
    onChapter: (chapter) {
      Navigator.of(ctx).pop();
      onChapter(chapter);
    },
    onSection: (chapter, section) {
      Navigator.of(ctx).pop();
      onSection(chapter, section);
    },
  ),
  desktopWidth: 440,
  desktopHeight: 560,
  shrink: false,
  mobileHeightFactor: .7,
);

PickerSheet _contents(
  BuildContext context, {
  required OutlineView outline,
  required OutlineWhere where,
  required ValueChanged<ChapterView> onChapter,
  required void Function(ChapterView chapter, SectionView section) onSection,
}) {
  final c = context.colors;
  final ty = context.type;
  final count = outline.chapters.length;
  return PickerSheet(
    key: const ValueKey('outline-contents'),
    listHeight: null,
    listPadding: const EdgeInsets.only(bottom: 12),
    listKey: const ValueKey('outline-contents-scroll'),
    header: SheetHeader(
      'Contents · $count ${count == 1 ? 'chapter' : 'chapters'}',
    ),
    list: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (ci, chapter) in outline.chapters.indexed) ...[
          MarkedRow(
            key: ValueKey('outline-contents-chapter-${chapter.id}'),
            onTap: () => onChapter(chapter),
            label: chapter.title,
            colour: outlineColor(context, ci),
            dot: true,
            small: true,
            oneLine: true,
            labelStyle: ts(ty.ui, c.textPrimary, weight: 600),
            padding: const EdgeInsets.fromLTRB(
              Layout.gutter,
              8,
              Layout.gutter,
              4,
            ),
            trailing: where.chapterId == chapter.id && where.sectionId == null
                ? _HereTag()
                : null,
            detail: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  [
                    if (chapter.when.isNotEmpty) chapter.when,
                    _countLine(chapter),
                  ].join(' · '),
                  style: ts(ty.uiSmall, c.textTertiary),
                ),
                if (chapter.decision != null) ...[
                  const SizedBox(height: 2),
                  DecisionLine(
                    chapter.decision!,
                    peek: true,
                    color: c.textSecondary,
                  ),
                ] else if (chapter.gist != null) ...[
                  const SizedBox(height: 2),
                  Text(
                    chapter.gist!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(ty.uiSmall, c.textSecondary),
                  ),
                ],
              ],
            ),
          ),
          for (final (si, section) in chapter.sections.indexed)
            MarkedRow(
              key: ValueKey('outline-contents-section-${section.id}'),
              onTap: () => onSection(chapter, section),
              label: section.title,
              colour: outlineColor(context, si + 1),
              small: true,
              oneLine: true,
              labelStyle: ts(
                ty.uiSmall,
                c.textPrimary,
                weight: where.sectionId == section.id ? 600 : 400,
              ),
              padding: const EdgeInsets.fromLTRB(
                Layout.gutter + 18,
                6,
                Layout.gutter,
                6,
              ),
              trailing: where.sectionId == section.id ? _HereTag() : null,
            ),
        ],
      ],
    ),
  );
}

class _HereTag extends StatelessWidget {
  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(left: 8),
    child: Text(
      "you're here",
      style: ts(context.type.uiSmall, context.colors.accentText),
    ),
  );
}

/// The menu of a chapter or a section: its actions as sheet rows. Each row
/// closes the sheet first.
Future<void> showOutlineMenu(
  BuildContext context, {
  required List<(String, Ico, VoidCallback)> actions,
}) => showSheet<void>(
  context,
  (ctx) => SheetSurface(
    children: [
      for (final (label, icon, action) in actions)
        SheetItem(
          label: label,
          icon: icon,
          onTap: () {
            Navigator.of(ctx).pop();
            action();
          },
        ),
    ],
  ),
);
