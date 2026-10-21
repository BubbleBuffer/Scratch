// The editor of text-like values (text, number, URL, email, phone, title):
// a box over the cell, grown to fit, that saves when it closes (click
// outside, Enter, Escape).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../registry/property_registry.dart';
import '../../../ui/ui.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';

/// Opens the text editor for [a]. [parse] turns the text into a value
/// (null: keep the old one); [initial] is the text to start with.
Future<void> editAsText(EditArgs a, {required String initial, required PropValue? Function(String s) parse, bool multiline = true}) async {
  String? result;
  await showOverCell<void>(
    a.context,
    a.anchor,
    (ctx) => _TextBox(
      initial: initial,
      multiline: multiline,
      onDone: (s) => result = s,
      placeholder: a.context.mobile ? a.def.name : '',
    ),
  );
  final s = result;
  if (s == null || s == initial) return;
  final v = parse(s);
  if (v != null) a.onChanged(v);
}

class _TextBox extends StatefulWidget {
  const _TextBox({required this.initial, required this.onDone, required this.multiline, required this.placeholder});
  final String initial;
  final ValueChanged<String> onDone;
  final bool multiline;
  final String placeholder;

  @override
  State<_TextBox> createState() => _TextBoxState();
}

class _TextBoxState extends State<_TextBox> {
  late final _ctl = TextEditingController(text: widget.initial)
    ..selection = TextSelection.collapsed(offset: widget.initial.length);
  late final _focus = FocusNode(onKeyEvent: _onKey);

  KeyEventResult _onKey(FocusNode n, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final enter = e.logicalKey == LogicalKeyboardKey.enter || e.logicalKey == LogicalKeyboardKey.numpadEnter;
    if (enter && !HardwareKeyboard.instance.isShiftPressed) {
      Navigator.of(context).maybePop();
      return KeyEventResult.handled;
    }
    if (e.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).maybePop();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  // The route's future completes when it starts closing, before this state
  // is disposed, so the text is reported as it changes.
  void _report() => widget.onDone(_ctl.text.replaceAll('\n', widget.multiline ? '\n' : ''));

  @override
  void initState() {
    super.initState();
    _ctl.addListener(_report);
  }

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final style = ts(m ? context.type.body : const TypeSpec(14, 21, 400), c.textPrimary, weight: 400);
    return Padding(
      padding: m ? const EdgeInsets.fromLTRB(8, 0, 8, 12) : const EdgeInsets.fromLTRB(8, 7.5, 8, 7.5),
      child: m
          ? MenuField(controller: _ctl, autofocus: true, placeholder: widget.placeholder, onSubmitted: (_) => Navigator.of(context).maybePop())
          : ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 21, maxHeight: 300),
              child: EditableText(
                key: const ValueKey('cell-editor'),
                controller: _ctl,
                focusNode: _focus,
                autofocus: true,
                style: style,
                cursorColor: c.textPrimary,
                backgroundCursorColor: c.textTertiary,
                selectionColor: c.textSelection,
                cursorWidth: 1.5,
                maxLines: null,
                keyboardType: TextInputType.multiline,
                selectionControls: noteSelectionControls,
              ),
            ),
    );
  }
}

/// Parses a number typed in a cell ("1,200", "12%", "$3"); null if none.
double? parseNumber(String s) {
  var t = s.trim().replaceAll(',', '').replaceAll(RegExp(r'[\$€£¥\s]'), '');
  var pct = false;
  if (t.endsWith('%')) {
    pct = true;
    t = t.substring(0, t.length - 1);
  }
  final v = double.tryParse(t);
  if (v == null) return null;
  return pct ? v / 100 : v;
}

/// A text value of [kind] from typed text (empty text clears it).
PropValue textValue(PropKind kind, String s) => Values.ofText(kind, s.trim());
