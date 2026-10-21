// The "Send feedback" sheet: a text field for what happened, the screenshot
// (removable) and the diagnostics that will be sent, collapsed by default.
// On a phone it is a bottom sheet, on desktop a dialog. It pops with true
// once [onSend] has saved the report.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../ui/buttons.dart';
import '../ui/fields.dart';
import '../ui/hover.dart';
import '../ui/icons.dart';
import '../ui/selection.dart';
import '../ui/sheet.dart';
import '../ui/tokens.dart';

class FeedbackSheet extends StatefulWidget {
  const FeedbackSheet({
    super.key,
    required this.screenshot,
    required this.diagnostics,
    required this.onSend,
  });

  /// The PNG to attach, or null when there is none.
  final Uint8List? screenshot;

  /// The text that will be sent after the typed words: already redacted.
  final String diagnostics;

  /// Saves the report. Completes with true when it was saved.
  final Future<bool> Function(String text, Uint8List? screenshot) onSend;

  @override
  State<FeedbackSheet> createState() => _FeedbackSheetState();
}

class _FeedbackSheetState extends State<FeedbackSheet> {
  final _text = TextEditingController();
  final _focus = FocusNode();
  late Uint8List? _screenshot = widget.screenshot;
  bool _detailsOpen = false;
  bool _sending = false;

  /// The phone sheet height outside its scrolling middle: the top and bottom
  /// padding, the handle, the title, the gaps and the buttons. Measured on the
  /// 375x812 shots; the bottom safe-area inset is taken off at build time.
  static const _phoneChrome = 134.0;

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    if (_sending) return;
    setState(() => _sending = true);
    final saved = await widget.onSend(_text.text, _screenshot);
    if (!mounted) return;
    if (saved) {
      Navigator.of(context).pop(true);
    } else {
      // The text and the screenshot stay as they were, for another try.
      setState(() => _sending = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final mobile = context.mobile;
    final title = Text(
      'Send feedback',
      style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary),
    );
    // The part that scrolls: the text, the screenshot and the diagnostics.
    final scrolling = Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 96),
          child: FieldFrame(
            child: Stack(
              children: [
                ValueListenableBuilder<TextEditingValue>(
                  valueListenable: _text,
                  builder: (context, value, _) => value.text.isEmpty
                      ? IgnorePointer(
                          child: Text(
                            'What happened?',
                            style: ts(ty.body, c.textTertiary),
                          ),
                        )
                      : const SizedBox.shrink(),
                ),
                EditableText(
                  key: const ValueKey('feedback-text'),
                  controller: _text,
                  focusNode: _focus,
                  style: ts(ty.body, c.textPrimary),
                  cursorColor: c.textPrimary,
                  backgroundCursorColor: c.textTertiary,
                  selectionColor: c.textSelection,
                  cursorWidth: 1.5,
                  autofocus: !mobile,
                  keyboardType: TextInputType.multiline,
                  textInputAction: TextInputAction.newline,
                  selectionControls: noteSelectionControls,
                  maxLines: null,
                ),
              ],
            ),
          ),
        ),
        if (_screenshot case final shot?) ...[
          const SizedBox(height: 12),
          _thumbnail(context, shot, maxHeight: mobile ? 140 : 120),
        ],
        const SizedBox(height: 8),
        Hover(
          key: const ValueKey('feedback-diagnostics-toggle'),
          onTap: () => setState(() => _detailsOpen = !_detailsOpen),
          borderRadius: Radii.ctl,
          child: SizedBox(
            height: 32,
            child: Row(
              children: [
                Icon20(
                  _detailsOpen ? Ico.chevronDown : Ico.chevronRight,
                  size: 16,
                  color: c.iconTertiary,
                ),
                const SizedBox(width: 6),
                Text('Diagnostics', style: ts(ty.uiSmall, c.textSecondary)),
              ],
            ),
          ),
        ),
        if (_detailsOpen)
          Container(
            key: const ValueKey('feedback-diagnostics'),
            constraints: const BoxConstraints(maxHeight: 240),
            width: double.infinity,
            padding: const EdgeInsets.all(10),
            decoration: BoxDecoration(
              color: c.bgSecondary,
              borderRadius: Radii.ctl,
            ),
            child: SingleChildScrollView(
              child: Text(
                widget.diagnostics,
                style: ts(ty.uiSmall, c.textSecondary, mono: true),
              ),
            ),
          ),
      ],
    );
    // Cancel and Send stay at the bottom, outside the scrolling part.
    final buttons = Row(
      children: [
        Expanded(
          child: SecondaryButton(
            key: const ValueKey('feedback-cancel'),
            label: 'Cancel',
            onTap: () => Navigator.of(context).pop(),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: PrimaryButton(
            key: const ValueKey('feedback-send'),
            label: 'Send',
            onTap: _send,
          ),
        ),
      ],
    );
    if (mobile) {
      // SheetSurface scrolls its children as one column, so the middle is
      // capped to the room the sheet has: the title and buttons stay in view.
      final media = MediaQuery.of(context);
      final room =
          media.size.height * 0.85 - _phoneChrome - media.padding.bottom;
      return SheetSurface(
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
            child: title,
          ),
          const SizedBox(height: 12),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
            child: ConstrainedBox(
              constraints: BoxConstraints(maxHeight: room),
              child: SingleChildScrollView(child: scrolling),
            ),
          ),
          const SizedBox(height: 16),
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Layout.gutter,
              0,
              Layout.gutter,
              8,
            ),
            child: buttons,
          ),
        ],
      );
    }
    // The dialog height is capped by its panel, so only the middle scrolls.
    return Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          title,
          const SizedBox(height: 12),
          Flexible(child: SingleChildScrollView(child: scrolling)),
          const SizedBox(height: 16),
          buttons,
        ],
      ),
    );
  }

  /// The screenshot at its own proportions, at most [maxHeight] tall, with a
  /// 1px border and the remove button in its top-right corner.
  Widget _thumbnail(
    BuildContext context,
    Uint8List shot, {
    required double maxHeight,
  }) {
    final c = context.colors;
    return Align(
      alignment: Alignment.centerLeft,
      child: Stack(
        children: [
          Container(
            padding: const EdgeInsets.all(1),
            decoration: BoxDecoration(
              border: Border.all(color: c.border),
              borderRadius: Radii.ctl,
            ),
            child: ClipRRect(
              borderRadius: Radii.ctl,
              child: ConstrainedBox(
                constraints: BoxConstraints(maxHeight: maxHeight),
                child: Image.memory(
                  shot,
                  key: const ValueKey('feedback-screenshot'),
                  fit: BoxFit.contain,
                ),
              ),
            ),
          ),
          Positioned(
            top: 4,
            right: 4,
            child: Hover(
              key: const ValueKey('feedback-remove-screenshot'),
              onTap: () => setState(() => _screenshot = null),
              borderRadius: Radii.ctl,
              background: c.bgPrimary,
              child: SizedBox(
                width: 28,
                height: 28,
                child: Center(
                  child: Icon20(Ico.close, size: 16, color: c.iconPrimary),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
