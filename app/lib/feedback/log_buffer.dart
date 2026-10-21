// In-memory ring buffer of the last app log lines and uncaught errors, for
// feedback reports. Pure Dart apart from the Flutter error hooks, so tests
// run without the Rust bridge.

import 'dart:collection';

import 'package:flutter/foundation.dart';

import '../core/clock.dart';

class LogLine {
  const LogLine(this.time, this.text);

  final DateTime time;
  final String text;
}

class LoggedError {
  const LoggedError({
    required this.time,
    required this.error,
    required this.stack,
    required this.source,
  });

  final DateTime time;
  final String error;
  final String stack;

  /// `flutter` for FlutterError.onError, `platform` for PlatformDispatcher.
  final String source;
}

class LogBuffer {
  LogBuffer({this.maxLines = 200, this.maxErrors = 20});

  /// The app-wide buffer that AppLog and the error hooks feed.
  static final LogBuffer instance = LogBuffer();

  final int maxLines;
  final int maxErrors;

  final _lines = Queue<LogLine>();
  final _errors = Queue<LoggedError>();

  /// Oldest first.
  List<LogLine> get lines => List.unmodifiable(_lines);

  /// Oldest first.
  List<LoggedError> get errors => List.unmodifiable(_errors);

  void addLine(String text) {
    _lines.addLast(LogLine(clockNow(), text));
    while (_lines.length > maxLines) {
      _lines.removeFirst();
    }
  }

  void addError(Object error, StackTrace? stack, {required String source}) {
    _errors.addLast(
      LoggedError(
        time: clockNow(),
        error: error.toString(),
        stack: (stack ?? StackTrace.empty).toString(),
        source: source,
      ),
    );
    while (_errors.length > maxErrors) {
      _errors.removeFirst();
    }
  }
}

void Function(FlutterErrorDetails)? _flutterHook;
bool Function(Object, StackTrace)? _platformHook;

/// Records uncaught Flutter and platform errors into [buffer] (default
/// [LogBuffer.instance]) and then hands each one to the handler that was
/// installed before, so default reporting still happens. Calling it again
/// while these hooks are still in place does nothing.
void installErrorHooks({LogBuffer? buffer}) {
  final target = buffer ?? LogBuffer.instance;

  if (_flutterHook == null || !identical(FlutterError.onError, _flutterHook)) {
    final previous = FlutterError.onError;
    _flutterHook = (details) {
      target.addError(details.exception, details.stack, source: 'flutter');
      if (previous != null) {
        previous(details);
      } else {
        FlutterError.presentError(details);
      }
    };
    FlutterError.onError = _flutterHook;
  }

  if (_platformHook == null ||
      !identical(PlatformDispatcher.instance.onError, _platformHook)) {
    final previous = PlatformDispatcher.instance.onError;
    _platformHook = (error, stack) {
      target.addError(error, stack, source: 'platform');
      return previous?.call(error, stack) ?? false;
    };
    PlatformDispatcher.instance.onError = _platformHook;
  }
}
