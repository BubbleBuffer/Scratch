// Where the reader is in the outlined transcript: the where-am-I strip over
// it and the flash that marks a row a jump has landed on.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';

/// Where the reader is: the chapter and section titles and ids at the top of
/// the transcript, or nulls above the first chapter.
typedef OutlineWhere = ({
  String? chapter,
  String? section,
  String? chapterId,
  String? sectionId,
});

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
