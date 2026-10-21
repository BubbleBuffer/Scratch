// Presentation pieces of the search card: the query text, the count, one
// result row and the empty line. Colours and type come from the kit tokens.
// Form B (inline chip) will reuse the row, the count and the highlighting.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../src/rust/api/types.dart';
import '../../ui/ui.dart';

/// Rows shown before "Show N more".
const queryPreviewRows = 5;

/// The highest count the core reports; above it the label reads "200+".
const queryCountCeiling = 201;

String queryCountLabel(int total) =>
    total >= queryCountCeiling ? '200+' : '$total';

/// Tags take the accent text colour and `key:` words are muted, as in the
/// approved mockup. Other words keep the base colour.
TextSpan highlightQuery(
  String query, {
  required TextStyle base,
  required Color tag,
  required Color key,
}) {
  final spans = <TextSpan>[];
  // Words and the whitespace between them, both kept so the query reads as typed.
  for (final match in RegExp(r'\S+|\s+').allMatches(query)) {
    final token = match.group(0)!;
    if (token.trim().isEmpty) {
      spans.add(TextSpan(text: token));
    } else if (token.startsWith('#')) {
      spans.add(TextSpan(text: token, style: base.copyWith(color: tag)));
    } else if (RegExp(r'^-?[A-Za-z]+:').hasMatch(token)) {
      final colon = token.indexOf(':') + 1;
      spans.add(
        TextSpan(
          children: [
            TextSpan(text: token.substring(0, colon), style: base.copyWith(color: key)),
            TextSpan(text: token.substring(colon)),
          ],
          style: base,
        ),
      );
    } else {
      spans.add(TextSpan(text: token));
    }
  }
  return TextSpan(style: base, children: spans);
}

/// Shows a visible focus ring and turns Enter or Space into [onActivate].
class FocusFrame extends StatefulWidget {
  const FocusFrame({
    super.key,
    required this.child,
    required this.onActivate,
    this.borderRadius = Radii.ctl,
  });

  final Widget child;
  final VoidCallback onActivate;
  final BorderRadius borderRadius;

  @override
  State<FocusFrame> createState() => _FocusFrameState();
}

class _FocusFrameState extends State<FocusFrame> {
  bool _focused = false;

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter ||
        key == LogicalKeyboardKey.space) {
      widget.onActivate();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    return Focus(
      onKeyEvent: _onKey,
      onFocusChange: (focused) => setState(() => _focused = focused),
      child: DecoratedBox(
        decoration: BoxDecoration(
          borderRadius: widget.borderRadius,
          border: Border.all(
            color: _focused ? context.colors.focus : context.colors.focus.withAlpha(0),
            width: 2,
          ),
        ),
        child: widget.child,
      ),
    );
  }
}

/// One result: a checkbox for a task (when Tasks is on), otherwise the page's
/// own icon. Tapping the row opens the page; the checkbox ticks the task.
class QueryRowTile extends StatelessWidget {
  const QueryRowTile({
    super.key,
    required this.hit,
    required this.onOpen,
    this.onTick,
    this.done = false,
    this.pageIcon,
    this.statusLabel,
  });

  final Hit hit;
  final VoidCallback onOpen;

  /// Non-null for a task row while Tasks is on: the checkbox calls it.
  final VoidCallback? onTick;

  /// Whether the task is done (the checkbox shows checked).
  final bool done;

  /// The page's own icon (an emoji), or null for the generic icon.
  final String? pageIcon;

  /// A status column's value, shown as a pill (see `statusLabelOf`).
  final String? statusLabel;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final mobile = context.mobile;
    final snippet = hit.snippet.trim();
    final tick = onTick;
    final leading = tick != null
        ? StacksCheckbox(
            key: ValueKey('query-check-${hit.id}'),
            checked: done,
            onChanged: (_) => tick(),
          )
        : PageIcon(
            key: ValueKey('query-page-${hit.id}'),
            icon: pageIcon,
            size: 20,
          );
    return FocusFrame(
      onActivate: tick ?? onOpen,
      borderRadius: BorderRadius.circular(Radii.sm),
      child: Hover(
        key: ValueKey('query-row-${hit.id}'),
        onTap: onOpen,
        borderRadius: BorderRadius.circular(Radii.sm),
        hoverBackground: c.bgHover,
        child: SizedBox(
          height: mobile ? 52 : 40,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Row(
              children: [
                SizedBox(width: 22, child: Center(child: leading)),
                const SizedBox(width: 10),
                // flex 0: the title takes its own width, so the snippet's
                // Expanded gets the rest and sits at the row's end.
                Flexible(
                  flex: 0,
                  child: Text(
                    hit.title.isEmpty ? 'Untitled' : hit.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(ty.body, c.textPrimary),
                  ),
                ),
                if (statusLabel != null) ...[
                  const SizedBox(width: 8),
                  QueryStatusPill(label: statusLabel!),
                ],
                if (snippet.isNotEmpty) ...[
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      snippet,
                      maxLines: 1,
                      textAlign: TextAlign.end,
                      overflow: TextOverflow.ellipsis,
                      style: ts(ty.uiSmall, c.textTertiary),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A board row's status ("Doing"). Pills are drawn only when [statusLabelOf]
/// returns a value.
class QueryStatusPill extends StatelessWidget {
  const QueryStatusPill({super.key, required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: c.borderSubtle),
      ),
      child: Text(label, style: ts(context.type.uiSmall, c.textSecondary)),
    );
  }
}

/// The one-line empty state, worded per query (see `queryWording`).
class QueryEmptyLine extends StatelessWidget {
  const QueryEmptyLine({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
      child: Row(
        children: [
          Icon20(Ico.search, size: 16, color: c.textTertiary),
          const SizedBox(width: 8),
          Expanded(
            child: Text(text, style: ts(context.type.body, c.textTertiary)),
          ),
        ],
      ),
    );
  }
}

/// The count at the header's right edge: muted when zero.
class QueryCountBadge extends StatelessWidget {
  const QueryCountBadge({super.key, required this.total});

  final int total;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Text(
      queryCountLabel(total),
      key: const ValueKey('query-count'),
      style: ts(
        context.type.uiSmall,
        total == 0 ? c.textTertiary : c.textSecondary,
        weight: 600,
      ),
    );
  }
}
