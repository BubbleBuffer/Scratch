// Text inputs: a single-line field with a placeholder, and the search pill.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'buttons.dart';
import 'icons.dart';
import 'selection.dart';
import 'tokens.dart';

/// A single-line text field with a placeholder, built on EditableText.
class PlainField extends StatefulWidget {
  const PlainField({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.style,
    required this.placeholder,
    this.onChanged,
    this.onSubmitted,
    this.autofocus = false,
    this.textInputAction = TextInputAction.search,
  });
  final TextEditingController controller;
  final FocusNode focusNode;
  final TextStyle style;
  final String placeholder;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;
  final bool autofocus;
  final TextInputAction textInputAction;

  @override
  State<PlainField> createState() => _PlainFieldState();
}

class _PlainFieldState extends State<PlainField> {
  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_changed);
  }

  @override
  void didUpdateWidget(PlainField old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_changed);
      widget.controller.addListener(_changed);
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    super.dispose();
  }

  void _changed() => setState(() {});

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Stack(
      children: [
        if (widget.controller.text.isEmpty)
          Positioned.fill(
            child: IgnorePointer(
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(
                  widget.placeholder,
                  maxLines: 1,
                  overflow: TextOverflow.clip,
                  style: widget.style.copyWith(color: c.textTertiary),
                ),
              ),
            ),
          ),
        EditableText(
          controller: widget.controller,
          focusNode: widget.focusNode,
          style: widget.style,
          cursorColor: c.textPrimary,
          backgroundCursorColor: c.textTertiary,
          selectionColor: c.textSelection,
          cursorWidth: 1.5,
          autofocus: widget.autofocus,
          onChanged: widget.onChanged,
          onSubmitted: widget.onSubmitted,
          selectionControls: noteSelectionControls,
          textInputAction: widget.textInputAction,
          maxLines: 1,
        ),
      ],
    );
  }
}

/// The search pill: magnifier, field, clear button.
class SearchField extends StatelessWidget {
  const SearchField({
    super.key,
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

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final mobile = context.mobile;
    return FieldFrame(
      height: mobile ? 40 : 32,
      padding: EdgeInsets.symmetric(horizontal: mobile ? 12 : 8),
      child: Row(
        children: [
          Icon20(Ico.search, size: mobile ? 20 : 16, color: c.iconTertiary),
          SizedBox(width: mobile ? 8 : 6),
          Expanded(
            child: PlainField(
              key: fieldKey,
              controller: controller,
              focusNode: focusNode,
              style: ts(mobile ? ty.body : ty.ui, c.textPrimary, weight: 400),
              placeholder: placeholder ?? (mobile ? 'Search' : 'Search pages…'),
              onChanged: onChanged,
            ),
          ),
          ListenableBuilder(
            listenable: controller,
            builder: (context, _) => controller.text.isEmpty
                ? const SizedBox.shrink()
                : IconBtn(
                    Ico.close,
                    onTap: () {
                      controller.clear();
                      onChanged('');
                    },
                    size: 24,
                    glyph: 14,
                    color: c.iconTertiary,
                    semanticLabel: 'Clear search',
                  ),
          ),
        ],
      ),
    );
  }
}

/// Border and offset focus ring for a field, without changing its contents.
class FieldFrame extends StatefulWidget {
  const FieldFrame({
    super.key,
    required this.child,
    this.height,
    this.padding = const EdgeInsets.symmetric(horizontal: 8),
  });
  final Widget child;
  final double? height;
  final EdgeInsetsGeometry padding;
  @override
  State<FieldFrame> createState() => _FieldFrameState();
}

class _FieldFrameState extends State<FieldFrame> {
  bool _hover = false, _focus = false;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Focus(
      onFocusChange: (value) => setState(() => _focus = value),
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: CustomPaint(
          foregroundPainter: _focus ? _FieldFocusRing(c.focus) : null,
          child: AnimatedContainer(
            duration: Motion.of(context).fast,
            curve: Motion.ease,
            height: widget.height,
            padding: widget.padding,
            alignment: Alignment.centerLeft,
            decoration: BoxDecoration(
              color: c.bgSecondary,
              borderRadius: Radii.field,
              border: Border.all(color: _hover ? c.threadStrong : c.border),
            ),
            child: widget.child,
          ),
        ),
      ),
    );
  }
}

class _FieldFocusRing extends CustomPainter {
  const _FieldFocusRing(this.color);
  final Color color;
  @override
  void paint(Canvas canvas, Size size) {
    final rect = (Offset.zero & size).inflate(3);
    canvas.drawRRect(
      Radii.field.toRRect(rect),
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2,
    );
  }

  @override
  bool shouldRepaint(_FieldFocusRing old) => old.color != color;
}
