// Headings 1 to 3: larger text with space above, placeholder always shown.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';
import '../ui/confetti.dart';

/// Optional editor projection; nothing is stored in the page's blocks.
abstract interface class HeadingProgressHost {
  List<Node> get headingProgressOrder;
  String? get headingProgressSpaceColor;
}

/// A user tick's revision, separate from the live progress projection.
abstract interface class HeadingCompletionHost {
  int headingCompletionRevision(String headingId);
}

class HeadingProgress {
  const HeadingProgress(this.done, this.total);
  final int done, total;
  double get fraction => done / total;
}

int? headingLevel(String kind) => switch (kind) {
  Kinds.heading1 || Kinds.toggleHeading1 => 1,
  Kinds.heading2 || Kinds.toggleHeading2 => 2,
  Kinds.heading3 || Kinds.toggleHeading3 => 3,
  _ => null,
};

/// Includes subordinate headings and hidden to-dos until this section ends.
HeadingProgress? headingProgressOf(Node heading, List<Node> order) {
  final level = headingLevel(heading.kind);
  final start = order.indexWhere((n) => n.id == heading.id);
  if (level == null || start < 0) return null;
  var done = 0, total = 0;
  for (final n in order.skip(start + 1)) {
    final next = headingLevel(n.kind);
    if (next != null && next <= level) break;
    if (n.kind == Kinds.todo) {
      total++;
      if (n.checked) done++;
    }
  }
  return total < 2 ? null : HeadingProgress(done, total);
}

Widget decorateHeadingProgress(BlockContext b, Widget text) {
  final host = b.host;
  if (host is! HeadingProgressHost) return text;
  final projection = host as HeadingProgressHost;
  final progress = headingProgressOf(b.node, projection.headingProgressOrder);
  if (progress == null) return text;
  final placeholder = b.node.kind.startsWith('toggle')
      ? 'Toggle heading ${headingLevel(b.node.kind)}'
      : 'Heading ${headingLevel(b.node.kind)}';
  final placeholderPainter = TextPainter(
    text: TextSpan(text: placeholder, style: b.style),
    textDirection: Directionality.of(b.context),
    textScaler: MediaQuery.textScalerOf(b.context),
  )..layout();
  final emptyWidth = placeholderPainter.width + 2;
  placeholderPainter.dispose();
  return LayoutBuilder(
    builder: (context, box) => Row(
      children: [
        Flexible(
          fit: FlexFit.loose,
          child: IntrinsicWidth(
            child: ConstrainedBox(
              constraints: BoxConstraints(
                minWidth: b.node.text.isEmpty ? emptyWidth : 0,
              ),
              child: text,
            ),
          ),
        ),
        const SizedBox(width: 12),
        SizedBox(
          width: (box.maxWidth * .32).clamp(0, 220),
          child: ConfettiBurst(
            key: ValueKey('heading-confetti-${b.node.id}'),
            trigger: host is HeadingCompletionHost
                ? (host as HeadingCompletionHost).headingCompletionRevision(
                    b.node.id,
                  )
                : 0,
            patch:
                spacePatch(
                  projection.headingProgressSpaceColor,
                  b.colors.dark ? Brightness.dark : Brightness.light,
                ) ??
                spacePatch(
                  'apricot',
                  b.colors.dark ? Brightness.dark : Brightness.light,
                )!,
            child: HeadingProgressBar(
              progress: progress,
              spaceName: projection.headingProgressSpaceColor,
            ),
          ),
        ),
        const SizedBox(width: 10),
        Text(
          progress.done == progress.total
              ? 'All done'
              : '${progress.done} of ${progress.total}',
          key: ValueKey('heading-progress-${b.node.id}'),
          style: tsRaw(13, 18, 400, b.colors.textSecondary),
        ),
      ],
    ),
  );
}

class HeadingProgressBar extends StatelessWidget {
  const HeadingProgressBar({super.key, required this.progress, this.spaceName});
  final HeadingProgress progress;
  final String? spaceName;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final fill = progress.done == progress.total
        ? c.accent
        : spaceColor(spaceName, c.dark ? Brightness.dark : Brightness.light) ??
              spaceColor('moss', Brightness.light)!;
    return Semantics(
      label: '${progress.done} of ${progress.total} tasks done',
      child: ClipRRect(
        borderRadius: Radii.pill,
        child: ColoredBox(
          color: c.border,
          child: SizedBox(
            height: 6,
            child: TweenAnimationBuilder<double>(
              tween: Tween(end: progress.fraction),
              duration: Motion.of(context).standard,
              curve: Motion.ease,
              builder: (context, fraction, _) => Align(
                alignment: Alignment.centerLeft,
                child: FractionallySizedBox(
                  widthFactor: fraction,
                  heightFactor: 1,
                  child: ColoredBox(color: fill),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

TextStyle _h1(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h1, c.textPrimary);
TextStyle _h2(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h2, c.textPrimary);
TextStyle _h3(Node n, AppColors c, TypeScale ty) =>
    tsDisplay(ty.h3, c.textPrimary);
double _m1(bool mobile) => mobile ? 20 : 32;
double _m2(bool mobile) => mobile ? 16 : 22;
double _m3(bool mobile) => mobile ? 12 : 16;

const heading1Block = BlockType(
  kind: Kinds.heading1,
  label: 'Heading 1',
  hint: 'Big section heading.',
  iconText: 'H1',
  aliases: 'heading1 h1 title',
  shortcut: '#',
  markdown: ['# '],
  placeholder: 'Heading 1',
  alwaysShowPlaceholder: true,
  textStyle: _h1,
  topMargin: _m1,
  decorate: decorateHeadingProgress,
);

const heading2Block = BlockType(
  kind: Kinds.heading2,
  label: 'Heading 2',
  hint: 'Medium section heading.',
  iconText: 'H2',
  aliases: 'heading2 h2 subtitle',
  shortcut: '##',
  markdown: ['## '],
  placeholder: 'Heading 2',
  alwaysShowPlaceholder: true,
  textStyle: _h2,
  topMargin: _m2,
  decorate: decorateHeadingProgress,
);

const heading3Block = BlockType(
  kind: Kinds.heading3,
  label: 'Heading 3',
  hint: 'Small section heading.',
  iconText: 'H3',
  aliases: 'heading3 h3',
  shortcut: '###',
  markdown: ['### '],
  placeholder: 'Heading 3',
  alwaysShowPlaceholder: true,
  textStyle: _h3,
  topMargin: _m3,
  decorate: decorateHeadingProgress,
);
