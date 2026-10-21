// A card that folds: a coloured edge, a label, a title and a peek line on its
// face; a down-chevron opens the body and the footer. A group stacks paper
// sheets behind its face. Generic: the caller supplies the text and actions.

import 'package:flutter/widgets.dart';

import 'hover.dart';
import 'icons.dart';
import 'tokens.dart';

/// A folding card. Folded it shows the label, title and [peek]; open it shows
/// [body]. [footer] sits below either state (return null to show nothing).
/// [stacked] draws two sheets behind the face. Open state is internal unless
/// [open] is given, then the caller owns it and [onOpenChanged] reports taps.
class FoldableStackCard extends StatefulWidget {
  const FoldableStackCard({
    super.key,
    required this.edge,
    required this.title,
    this.label,
    this.faceKey,
    this.peek,
    this.body,
    this.footer,
    this.stacked = false,
    this.initiallyOpen = false,
    this.open,
    this.onOpenChanged,
    this.titleMaxLines = 2,
  });

  /// The colour of the left edge.
  final Color edge;

  /// Small text above the title, such as a breadcrumb; one line.
  final String? label;
  final String title;

  /// The caller chooses how many lines the title can occupy.
  final int titleMaxLines;

  /// Key for the fold header, the part that toggles the card.
  final Key? faceKey;

  /// Shown under the title while folded.
  final Widget? peek;

  /// Shown while open, under the face.
  final Widget? body;

  /// Shown under the body in both states when it returns a widget. Gets the
  /// open state so an action can appear only while open.
  final Widget? Function(bool open)? footer;

  /// Two paper sheets behind the face (a group).
  final bool stacked;
  final bool initiallyOpen;

  /// When non-null the caller owns the open state; tapping only reports it.
  final bool? open;
  final ValueChanged<bool>? onOpenChanged;

  @override
  State<FoldableStackCard> createState() => _FoldableStackCardState();
}

class _FoldableStackCardState extends State<FoldableStackCard> {
  late bool _openState = widget.initiallyOpen;

  bool get _isOpen => widget.open ?? _openState;

  void _toggle() {
    final next = !_isOpen;
    if (widget.open == null) setState(() => _openState = next);
    widget.onOpenChanged?.call(next);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final open = _isOpen;

    final header = Hover(
      key: widget.faceKey,
      radius: Radii.md,
      onTap: _toggle,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (widget.label != null) ...[
                  Text(
                    widget.label!,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(ty.uiSmall, c.textTertiary),
                  ),
                  const SizedBox(height: 1),
                ],
                Text(
                  widget.title,
                  maxLines: widget.titleMaxLines,
                  overflow: TextOverflow.ellipsis,
                  style: tsDisplay(ty.h3, c.textPrimary),
                ),
                if (!open && widget.peek != null) ...[
                  const SizedBox(height: 3),
                  widget.peek!,
                ],
              ],
            ),
          ),
          const SizedBox(width: 10),
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: AnimatedRotation(
              turns: open ? .5 : 0,
              duration: Motion.standard,
              child: Container(
                width: 26,
                height: 26,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  borderRadius: Radii.ctl,
                  border: Border.all(color: c.border),
                ),
                child: Icon20(
                  Ico.chevronDown,
                  size: 14,
                  color: c.iconSecondary,
                ),
              ),
            ),
          ),
        ],
      ),
    );

    final body = widget.body;
    final foot = widget.footer?.call(open);
    final below = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (open && body != null)
          Padding(padding: const EdgeInsets.only(top: 8), child: body),
        ?foot,
      ],
    );

    final face = ClipRRect(
      borderRadius: BorderRadius.circular(Radii.lg),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: c.bgElevated,
          borderRadius: BorderRadius.circular(Radii.lg),
          border: Border.all(color: c.border),
        ),
        child: Stack(
          children: [
            Positioned(
              left: 0,
              top: 0,
              bottom: 0,
              width: 4,
              child: ColoredBox(color: widget.edge),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 10, 12, 10),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  header,
                  AnimatedSize(
                    duration: Motion.standard,
                    curve: Motion.ease,
                    alignment: Alignment.topCenter,
                    child: below,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );

    return Padding(
      padding: EdgeInsets.only(top: widget.stacked ? 10 : 0, bottom: 6),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          // Later children paint on top, so the farthest sheet comes first.
          if (widget.stacked) ...[
            Positioned(
              top: -10,
              left: 13,
              right: 13,
              height: 30,
              child: _PaperSheet(colour: c.bgTertiary, border: c.border),
            ),
            Positioned(
              top: -5,
              left: 6,
              right: 6,
              height: 30,
              child: _PaperSheet(colour: c.bgSecondary, border: c.border),
            ),
          ],
          face,
        ],
      ),
    );
  }
}

class _PaperSheet extends StatelessWidget {
  const _PaperSheet({required this.colour, required this.border});
  final Color colour;
  final Color border;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: colour,
      borderRadius: BorderRadius.circular(Radii.lg),
      border: Border.all(color: border),
    ),
  );
}

/// A quiet text action for a card's foot: a hairline frame, filled with the
/// accent when [main]. A null [onTap] shows it inert.
class FooterAction extends StatelessWidget {
  const FooterAction({
    super.key,
    required this.label,
    required this.onTap,
    this.main = false,
  });
  final String label;
  final VoidCallback? onTap;
  final bool main;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      onTap: onTap,
      radius: Radii.md,
      border: Border.all(color: main ? c.accent : c.borderSubtle),
      background: main ? c.accent : null,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
      child: Text(
        label,
        style: ts(context.type.uiSmall, main ? c.onAccent : c.textPrimary),
      ),
    );
  }
}
