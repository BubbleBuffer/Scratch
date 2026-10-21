// Rows that start with a colour mark: a short bar (a section) or a dot (a
// chapter). A row with [onTap] is a tappable row; a list can rule its rows.

import 'package:flutter/widgets.dart';

import 'hover.dart';
import 'tokens.dart';

/// One row: the mark, the [label], an optional [trailing] widget and a
/// [detail] line under the label (indented to the label). [oneLine] clips the
/// label with an ellipsis.
class MarkedRow extends StatelessWidget {
  const MarkedRow({
    super.key,
    required this.label,
    required this.colour,
    this.onTap,
    this.dot = false,
    this.small = false,
    this.oneLine = false,
    this.labelStyle,
    this.trailing,
    this.detail,
    this.padding = const EdgeInsets.symmetric(vertical: 5),
  });
  final String label;
  final Color colour;
  final VoidCallback? onTap;

  /// A round mark instead of a bar.
  final bool dot;

  /// The smaller interface size for the label.
  final bool small;
  final bool oneLine;

  /// Overrides the label's style (weight, colour).
  final TextStyle? labelStyle;
  final Widget? trailing;
  final Widget? detail;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final mark = dot
        ? Container(
            width: 9,
            height: 9,
            decoration: BoxDecoration(color: colour, shape: BoxShape.circle),
          )
        : Container(
            width: 4,
            height: 12,
            decoration: BoxDecoration(
              color: colour,
              borderRadius: BorderRadius.circular(2),
            ),
          );
    final content = Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // Centred on the first line of text (a line is about 20 px).
        Padding(
          padding: EdgeInsets.only(top: dot ? 5 : 4),
          child: mark,
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      label,
                      maxLines: oneLine ? 1 : null,
                      overflow: oneLine
                          ? TextOverflow.ellipsis
                          : TextOverflow.clip,
                      style:
                          labelStyle ??
                          ts(
                            small ? ty.uiSmall : ty.ui,
                            c.textPrimary,
                          ),
                    ),
                  ),
                  ?trailing,
                ],
              ),
              ?detail,
            ],
          ),
        ),
      ],
    );
    if (onTap == null) {
      return Padding(padding: padding, child: content);
    }
    return Hover(
      onTap: onTap,
      radius: Radii.md,
      padding: padding,
      child: content,
    );
  }
}

/// A column of [MarkedRow]s. [divided] rules a hairline above each row.
class MarkedList extends StatelessWidget {
  const MarkedList({super.key, required this.rows, this.divided = false});
  final List<MarkedRow> rows;
  final bool divided;

  @override
  Widget build(BuildContext context) {
    final rule = context.colors.borderSubtle;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final row in rows)
          divided
              ? DecoratedBox(
                  decoration: BoxDecoration(
                    border: Border(top: BorderSide(color: rule)),
                  ),
                  child: row,
                )
              : row,
      ],
    );
  }
}
