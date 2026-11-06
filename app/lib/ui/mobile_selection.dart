// Native text selection on phones: the selection handles keep the app's
// accent look (NoteSelectionControls), and the toolbar is Flutter's stock
// one for the platform (cut, copy, paste, select all, and on Android share and
// the system text-processing actions). Desktop keeps its own toolbar, so
// everything here is chosen with context.mobile.
//
// The app is a WidgetsApp with no Material ancestor, and the stock Android
// toolbar reads Material localizations and a Theme. Only the context menu is
// wrapped in them, themed from the app's colour tokens.

import 'package:flutter/material.dart'
    show
        AdaptiveTextSelectionToolbar,
        ColorScheme,
        DefaultMaterialLocalizations,
        Theme,
        ThemeData;
import 'package:flutter/widgets.dart';

import 'selection.dart';
import 'tokens.dart';

/// The app's handles, with the toolbar left to EditableText.contextMenuBuilder.
/// Without the mixin Flutter ignores the context menu builder and calls the
/// kit's empty buildToolbar instead.
class MobileSelectionControls extends NoteSelectionControls
    with TextSelectionHandleControls {}

final mobileSelectionControls = MobileSelectionControls();

/// The selection controls for a field: native on mobile, the kit's otherwise.
TextSelectionControls selectionControlsFor(BuildContext context) =>
    context.mobile ? mobileSelectionControls : noteSelectionControls;

/// The context menu for a field: the stock toolbar on mobile, none otherwise.
EditableTextContextMenuBuilder? contextMenuFor(BuildContext context) =>
    context.mobile ? mobileContextMenu : null;

/// The stock selection toolbar for [state], in the app's colours.
Widget mobileContextMenu(BuildContext context, EditableTextState state) {
  final c = context.colors;
  final scheme = ColorScheme(
    brightness: c.dark ? Brightness.dark : Brightness.light,
    primary: c.accent,
    onPrimary: c.bgPrimary,
    secondary: c.accent,
    onSecondary: c.bgPrimary,
    error: c.dangerBg,
    onError: c.bgPrimary,
    surface: c.bgElevated,
    onSurface: c.textPrimary,
  );
  return Localizations(
    locale: Localizations.localeOf(context),
    delegates: const [
      DefaultWidgetsLocalizations.delegate,
      DefaultMaterialLocalizations.delegate,
    ],
    child: Theme(
      data: ThemeData(colorScheme: scheme),
      child: AdaptiveTextSelectionToolbar.editableText(editableTextState: state),
    ),
  );
}

/// Wraps a plain EditableText (one without the editor's gesture detector) so
/// that a long press or double tap which selects text also shows the toolbar.
/// Pass the callback to the field's onSelectionChanged.
class SelectionToolbarHost extends StatefulWidget {
  const SelectionToolbarHost({super.key, required this.builder});
  final Widget Function(BuildContext context, SelectionChangedCallback onChanged) builder;

  @override
  State<SelectionToolbarHost> createState() => _SelectionToolbarHostState();
}

class _SelectionToolbarHostState extends State<SelectionToolbarHost> {
  void _changed(TextSelection sel, SelectionChangedCause? cause) {
    if (!context.mobile || sel.isCollapsed) return;
    if (cause != SelectionChangedCause.longPress &&
        cause != SelectionChangedCause.doubleTap) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _find(context as Element)?.showToolbar();
    });
  }

  EditableTextState? _find(Element root) {
    EditableTextState? found;
    void visit(Element e) {
      if (found != null) return;
      if (e is StatefulElement && e.state is EditableTextState) {
        found = e.state as EditableTextState;
        return;
      }
      e.visitChildren(visit);
    }

    root.visitChildren(visit);
    return found;
  }

  @override
  Widget build(BuildContext context) => widget.builder(context, _changed);
}
