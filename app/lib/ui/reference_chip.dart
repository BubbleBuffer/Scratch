// An inline reference chip: a colour dot and a short label, set in a sentence.

import 'package:flutter/widgets.dart';

import 'hover.dart';
import 'tokens.dart';

/// A small label with a colour dot, for a place in running text. Tapping it
/// (when [onTap] is set) is up to the caller.
class ReferenceChip extends StatelessWidget {
  const ReferenceChip({
    super.key,
    this.label,
    this.segments = const [],
    required this.colour,
    this.onTap,
  });
  final String? label;
  final List<String> segments;
  final Color colour;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      radius: Radii.sm,
      onTap: onTap,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      background: c.bgSecondary,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(color: colour, shape: BoxShape.circle),
          ),
          const SizedBox(width: 6),
          Flexible(
            child: Text(
              label ?? segments.join(' › '),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textSecondary),
            ),
          ),
        ],
      ),
    );
  }
}
