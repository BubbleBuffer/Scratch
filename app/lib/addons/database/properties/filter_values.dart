// Filter rule value inputs shared by the property types: a text or number
// field, an option list, a date.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart' hide StatusGroup;
import '../../../ui/database/config.dart';
import '../editors/date_picker.dart';
import '../editors/text_editor.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';
import '../../../registry/property_registry.dart';
import '../../../ui/ui.dart';

Widget textFilterValue(FilterValueArgs a) => MenuField(
      key: ValueKey('filter-value-${a.def.key}'),
      initial: a.value.kind == ValueKind.empty ? '' : a.value.text,
      placeholder: 'Type a value…',
      autofocus: !a.context.mobile,
      onChanged: (s) => a.onChanged(Values.text(s)),
    );

Widget numberFilterValue(FilterValueArgs a) => MenuField(
      key: ValueKey('filter-value-${a.def.key}'),
      initial: a.value.kind == ValueKind.number ? _num(a.value.number) : '',
      placeholder: 'Type a value…',
      autofocus: !a.context.mobile,
      onChanged: (s) {
        final n = parseNumber(s);
        a.onChanged(n == null ? Values.empty : Values.number(n));
      },
    );

String _num(double v) => v == v.roundToDouble() ? v.toInt().toString() : '$v';

/// The options to pick one from (select, status: Is; multi-select:
/// Contains).
Widget optionFilterValue(FilterValueArgs a) => _OptionFilter(a);

class _OptionFilter extends StatelessWidget {
  const _OptionFilter(this.a);
  final FilterValueArgs a;

  @override
  Widget build(BuildContext context) {
    final picked = a.value.optionIds.toSet();
    final c = context.colors;
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final o in a.def.options)
        MenuItem(
          key: ValueKey('filter-option-${o.name}'),
          label: '',
          leading: Pill(o.name, color: o.color, status: a.def.kind == PropKind.status,
              group: a.def.kind == PropKind.status
                  ? statusGroupOf(a.def, o.id)
                  : null),
          selected: picked.contains(o.id),
          trailing: picked.contains(o.id) ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
          onTap: () => a.onChanged(switch (a.def.kind) {
            PropKind.multiSelect => Values.multi([o.id]),
            PropKind.status => Values.status(o.id),
            _ => Values.select(o.id),
          }),
        ),
    ]);
  }
}

Widget dateFilterValue(FilterValueArgs a) => DatePicker(
      args: EditArgs(
        context: a.context,
        def: a.def,
        value: a.value,
        anchor: Rect.zero,
        onChanged: (v) => a.onChanged(v.kind == ValueKind.date ? Values.date(v.startMs, hasTime: v.hasTime) : v),
        saveDef: (d) async => d,
        collection: '',
      ),
    );
