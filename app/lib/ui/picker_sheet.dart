// A picker's content: an optional header, an optional search field, a list
// that scrolls, and an optional foot. showPickerSheet shows it as a bottom
// sheet on a phone and a centred panel elsewhere.

import 'package:flutter/widgets.dart';

import 'fields.dart';
import 'modal.dart';
import 'sheet.dart';
import 'tokens.dart';

/// The search field of a [PickerSheet]: the caller keeps the controller and
/// focus node and filters the list in [onChanged].
class PickerSearch {
  const PickerSearch({
    required this.controller,
    required this.focusNode,
    required this.onChanged,
    this.placeholder,
    this.fieldKey,
  });
  final TextEditingController controller;
  final FocusNode focusNode;
  final ValueChanged<String> onChanged;
  final String? placeholder;
  final Key? fieldKey;
}

/// A group of sheet rows under a grey label, as the pickers lay them out.
class PickerGroup extends StatelessWidget {
  const PickerGroup({super.key, required this.label, required this.children});
  final String label;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [SheetHeader(label), ...children],
  );
}

/// The picker: [header] (a title or a SheetHeader), [above] (for example a
/// group of options), the optional [search], then [list] in a scrolling area,
/// then [footer] on a hairline. [listHeight] caps the list; null lets it fill
/// the height the surface gives it.
class PickerSheet extends StatelessWidget {
  const PickerSheet({
    super.key,
    required this.list,
    this.header,
    this.above,
    this.search,
    this.listHeight = 360,
    this.listPadding = const EdgeInsets.fromLTRB(8, 0, 8, 12),
    this.listKey,
    this.footer,
  });
  final Widget? header;
  final Widget? above;
  final PickerSearch? search;
  final Widget list;

  /// The list's largest height, or null to fill the space given.
  final double? listHeight;
  final EdgeInsetsGeometry listPadding;

  /// Key on the scrolling list, for tests.
  final Key? listKey;
  final Widget? footer;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final scroll = SingleChildScrollView(
      key: listKey,
      padding: listPadding,
      child: list,
    );
    final s = search;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ?header,
        ?above,
        if (s != null) ...[
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
            child: SearchField(
              controller: s.controller,
              focusNode: s.focusNode,
              onChanged: s.onChanged,
              placeholder: s.placeholder,
              fieldKey: s.fieldKey,
            ),
          ),
          const SizedBox(height: 4),
        ],
        if (listHeight == null)
          Flexible(child: scroll)
        else
          ConstrainedBox(
            constraints: BoxConstraints(maxHeight: listHeight!),
            child: scroll,
          ),
        if (footer != null)
          Container(
            padding: const EdgeInsets.fromLTRB(
              Layout.gutter,
              10,
              Layout.gutter,
              14,
            ),
            decoration: BoxDecoration(
              border: Border(top: BorderSide(color: c.divider)),
            ),
            child: footer,
          ),
      ],
    );
  }
}

/// Shows [builder]'s picker: a bottom sheet on a phone (the list area is
/// [mobileHeightFactor] of the screen when set), a centred panel elsewhere
/// ([desktopWidth], [desktopHeight]; [shrink] fits the panel to its content).
/// [desktopScroll] scrolls the content inside the panel. Completes with what
/// the picker pops.
Future<T?> showPickerSheet<T>(
  BuildContext context,
  WidgetBuilder builder, {
  double desktopWidth = 480,
  double desktopHeight = 640,
  bool shrink = true,
  bool desktopScroll = false,
  double? mobileHeightFactor,
}) {
  if (context.mobile) {
    return showSheet<T>(
      context,
      (ctx) => SheetSurface(
        children: [
          mobileHeightFactor == null
              ? builder(ctx)
              : SizedBox(
                  height: MediaQuery.sizeOf(ctx).height * mobileHeightFactor,
                  child: builder(ctx),
                ),
        ],
      ),
    );
  }
  return showModalPanel<T>(
    context,
    desktopScroll
        ? (ctx) => SingleChildScrollView(child: builder(ctx))
        : builder,
    width: desktopWidth,
    height: desktopHeight,
    shrink: shrink,
  );
}
