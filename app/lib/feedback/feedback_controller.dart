// Opens the "Send feedback" sheet and saves what it sends. The screenshot is
// taken before the sheet covers the screen, and the diagnostics are read from
// the app's state at that moment. Saving writes a report page under the hidden
// Feedback page; the report's blocks go in one transaction.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/vault.dart';
import '../features/ai/ai_store.dart';
import '../shell/page_stack.dart';
import '../state/app_state.dart';
import '../ui/modal.dart';
import '../ui/sheet.dart';
import '../ui/tokens.dart';
import '../ui/toast.dart';
import 'diagnostics_collector.dart';
import 'feedback_sheet.dart';
import 'log_buffer.dart';
import 'report.dart';

class FeedbackController {
  FeedbackController({required this.captureKey});

  /// Wraps the whole app: the screenshot is taken from it.
  final GlobalKey captureKey;

  bool _isOpen = false;

  /// True while the sheet is up. A second open does nothing meanwhile.
  bool get isOpen => _isOpen;

  /// Shows the sheet over the navigator that [context] belongs to.
  void open(BuildContext context) {
    if (_isOpen) return;
    _isOpen = true;
    unawaited(_show(context));
  }

  Future<void> _show(BuildContext context) async {
    try {
      final app = context.app;
      final media = MediaQuery.of(context);
      final settings = app.settings;
      final sync = app.sync;
      final status = sync.status;
      final stack = PageStackScope.maybeOf(context);
      final route = stack?.currentRoute;
      final routeName = route?.settings.name ?? '';
      final pageId = routeName == '/page'
          ? route?.settings.arguments as String?
          : null;
      final pageTitle = pageId == null ? null : stack?.titles[pageId];
      final secrets = [
        ?settings.clientToken,
        AiStore.of(app).settings.deepSeekKey,
      ];

      // The colours the app is drawn with: the setting, or the system (and NOTES_THEME).
      final dark = identical(AppTheme.of(context).colors, AppColors.darkTheme);
      final screenshot = await _capture(context);
      final pageCount = await app.vault.pageCount();
      final lastOk = status.lastOkMs;
      final diagnostics = await collectDiagnostics(
        screenSize: media.size,
        pixelRatio: media.devicePixelRatio,
        route: routeName,
        pageId: pageId,
        pageTitle: pageTitle,
        themeSetting: settings.theme.name,
        effectiveDark: dark,
        signedIn: settings.clientToken != null,
        hubUrl: settings.clientUrl,
        lastSync: lastOk == null
            ? null
            : DateTime.fromMillisecondsSinceEpoch(lastOk.toInt()),
        lastError: status.lastError,
        scheduleRunner: settings.scheduleRunsHere,
        pageCount: pageCount,
        logs: [
          for (final l in LogBuffer.instance.lines)
            '${l.time.toIso8601String()} ${l.text}',
        ],
        errors: [
          for (final e in LogBuffer.instance.errors)
            '${e.time.toIso8601String()} [${e.source}] ${e.error}\n${e.stack}',
        ],
      );
      if (!context.mounted) return;

      final vault = app.vault;
      final sent = redact(formatDiagnostics(diagnostics), secrets);
      final saved = await _showSheet<bool>(
        context,
        (ctx) => FeedbackSheet(
          screenshot: screenshot,
          diagnostics: sent,
          onSend: (text, shot) => _save(
            context,
            vault: vault,
            text: text,
            screenshot: shot,
            diagnostics: diagnostics,
            secrets: secrets,
          ),
        ),
      );
      if (saved == true && context.mounted) {
        showToast(context, 'Feedback saved — thanks');
      }
    } finally {
      _isOpen = false;
    }
  }

  Future<T?> _showSheet<T>(BuildContext context, WidgetBuilder builder) =>
      context.mobile
      ? showSheet<T>(context, builder)
      : showModalPanel<T>(context, builder, width: 560, shrink: true);

  /// The app's pixels as PNG, or null when there is no picture to take.
  Future<Uint8List?> _capture(BuildContext context) async {
    try {
      final box = captureKey.currentContext?.findRenderObject();
      if (box is! RenderRepaintBoundary) return null;
      final ratio = math.min(MediaQuery.devicePixelRatioOf(context), 2.0);
      final image = await box.toImage(pixelRatio: ratio);
      try {
        final data = await image.toByteData(format: ui.ImageByteFormat.png);
        return data?.buffer.asUint8List();
      } finally {
        image.dispose();
      }
    } catch (e) {
      LogBuffer.instance.addLine('feedback: no screenshot ($e)');
      return null;
    }
  }

  /// Writes the report page. Returns false, with an error toast, when the
  /// vault refuses; the sheet then stays open with its text.
  Future<bool> _save(
    BuildContext context, {
    required Vault vault,
    required String text,
    required Uint8List? screenshot,
    required Diagnostics diagnostics,
    required List<String> secrets,
  }) async {
    final now = clockNow();
    try {
      final feedbackId = await vault.run((v) => v.feedbackPage(), writes: true);
      String? screenshotId;
      if (screenshot != null) {
        screenshotId = await vault.run(
          (v) => v.addAttachmentBytes(
            name: 'feedback-${now.millisecondsSinceEpoch}.png',
            bytes: screenshot,
          ),
          writes: true,
        );
      }
      final report = buildFeedbackReport(
        now: now,
        text: text,
        screenshotId: screenshotId,
        diagnostics: diagnostics,
        secrets: secrets,
      );
      final pageId = await vault.createPage(
        parent: feedbackId,
        title: report.title,
      );
      try {
        await vault.apply(pageId, _reportOps(report.blocks));
      } catch (_) {
        // The page must not stay behind empty: a retry would add a second one.
        unawaited(vault.deletePage(pageId).catchError((_) {}));
        rethrow;
      }
      return true;
    } catch (_) {
      if (context.mounted) showErrorToast(context, 'Could not save feedback');
      return false;
    }
  }

  /// Insert ops for the report's blocks, in order, as top-level blocks of the
  /// report page. Images and code blocks get their target as a second op.
  List<BOp> _reportOps(List<ReportBlock> blocks) {
    final ops = <BOp>[];
    String? previous;
    for (final b in blocks) {
      final id = newId();
      final kind = switch (b.kind) {
        'image' => Kinds.image,
        'code' => Kinds.code,
        _ => Kinds.paragraph,
      };
      ops.add(
        BOp(
          op: OpKind.insert,
          id: id,
          after: previous,
          kind: kind,
          text: kind == Kinds.image ? '' : b.text,
          marks: const [],
          checked: false,
        ),
      );
      if (b.target.isNotEmpty) {
        ops.add(
          BOp(
            op: OpKind.setTarget,
            id: id,
            kind: Kinds.paragraph,
            text: '',
            marks: const [],
            checked: false,
            target: b.target,
          ),
        );
      }
      previous = id;
    }
    return ops;
  }
}
