// The type filter chips of the search panel: All, Pages and one per registered
// source, plus a removable chip for each other `type:` value in the query.

import 'package:flutter/widgets.dart';

import '../ui/ui.dart';
import 'addons.dart';
import 'search_groups.dart';

/// The label of a dynamic type chip: the plural of a known type, else the
/// value as typed.
String typeChipLabel(String value) => switch (value) {
  'document' => 'Documents',
  'database' => 'Databases',
  _ =>
    appAddonContext.searchSources.all
            .where((source) => source.noteTypes.contains(value))
            .firstOrNull
            ?.label ??
        value,
};

/// The type filter chips. All, Pages and Chats are fixed: each is selected
/// when the query's type values are exactly its set. A query whose type
/// values match no fixed set gets one selected, removable chip per value
/// (`type:document` shows "Documents"). [onPick] receives the type values to
/// write (empty for All).
class SearchTypeChips extends StatelessWidget {
  const SearchTypeChips({
    super.key,
    required this.active,
    required this.onPick,
  });
  final Set<String> active;
  final ValueChanged<Set<String>> onPick;

  @override
  Widget build(BuildContext context) {
    final fixed = <(String, String, Set<String>)>[
      ('all', 'All', const {}),
      ('pages', 'Pages', searchTypePages),
      for (final source in appAddonContext.searchSources.all)
        if (source.noteTypes.isNotEmpty)
          (source.groupId, source.label, source.noteTypes),
    ];
    final matchesFixed = fixed.any((f) => _sameTypes(active, f.$3));
    final dynamicValues = matchesFixed
        ? const <String>[]
        : (active.toList()..sort());
    return Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        for (final (id, label, types) in fixed)
          _TypeChip(
            key: ValueKey('search-type-$id'),
            label: label,
            selected: types.isEmpty
                ? active.isEmpty
                : _sameTypes(active, types),
            onTap: () {
              final selected = types.isEmpty
                  ? active.isEmpty
                  : _sameTypes(active, types);
              // Tapping the selected chip returns to All.
              onPick(selected ? const {} : types);
            },
          ),
        for (final value in dynamicValues)
          _TypeChip(
            key: ValueKey('search-type-value-$value'),
            label: typeChipLabel(value),
            selected: true,
            removable: true,
            // Removing a dynamic chip drops only its own value.
            onTap: () => onPick({
              for (final v in active)
                if (v != value) v,
            }),
          ),
      ],
    );
  }
}

bool _sameTypes(Set<String> a, Set<String> b) =>
    a.length == b.length && a.containsAll(b);

class _TypeChip extends StatelessWidget {
  const _TypeChip({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
    this.removable = false,
  });
  final String label;
  final bool selected;
  final VoidCallback onTap;

  /// Whether the chip ends with a close mark (its tap removes the value).
  final bool removable;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final ink = selected ? c.bgPrimary : c.textSecondary;
    return Semantics(
      button: true,
      selected: selected,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        // No alignment here: an aligned Container fills the Wrap's width, and
        // each chip would then take its own row.
        child: Container(
          constraints: BoxConstraints(minHeight: m ? Layout.minTapTarget : 24),
          padding: EdgeInsets.symmetric(horizontal: m ? 12 : 10),
          decoration: BoxDecoration(
            color: selected ? c.textPrimary : null,
            border: Border.all(color: selected ? c.textPrimary : c.border),
            borderRadius: Radii.pill,
          ),
          // Centred at the content's own width, so the pill keeps its minimum
          // height without stretching across the Wrap.
          child: Center(
            widthFactor: 1,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  label,
                  style: ts(m ? ty.ui : ty.uiSmall, ink, weight: 500),
                ),
                if (removable) ...[
                  const SizedBox(width: 4),
                  Icon20(Ico.close, size: 12, color: ink),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
