import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

// Keep screenshots/ for golden baselines. Failed comparisons belong in build/.
class BuildGoldenComparator extends LocalFileComparator {
  BuildGoldenComparator(super.testFile);

  @override
  Future<String> generateFailureOutput(
    ComparisonResult result,
    Uri golden,
    Uri basedir, {
    String key = '',
  }) => super.generateFailureOutput(
    result,
    golden,
    Directory('build/golden-diffs').absolute.uri,
    key: key,
  );
}

Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  final comparator = goldenFileComparator;
  if (comparator is LocalFileComparator) {
    goldenFileComparator = BuildGoldenComparator(
      comparator.basedir.resolve('flutter_test_config.dart'),
    );
  }
  await testMain();
}
