ObjC.import('AppKit');
ObjC.import('PDFKit');
ObjC.import('CoreGraphics');

function run(argv) {
  try {
    var url = $.NSURL.fileURLWithPath(argv[0]);
    var document = $.PDFDocument.alloc.initWithURL(url);
    if (document.isNil()) return 'open-failed';
    if (document.isEncrypted) return 'encrypted';
    if (!document.allowsPrinting) return 'denied';
    var session = ObjC.castRefToObject($.CGSessionCopyCurrentDictionary());
    if (session.isNil() || !ObjC.unwrap(
      session.objectForKey('kCGSessionLoginDoneKey')
    )) return 'unavailable';
    var app = $.NSApplication.sharedApplication;
    var operation = document.printOperationForPrintInfoScalingModeAutoRotate(
      $.NSPrintInfo.sharedPrintInfo, $.kPDFPrintPageScaleDownToFit, true
    );
    if (operation.isNil()) return 'failed';
    operation.jobTitle = url.lastPathComponent;
    operation.showsPrintPanel = true;
    app.setActivationPolicy($.NSApplicationActivationPolicyAccessory);
    app.activateIgnoringOtherApps(true);
    if (operation.runOperation) return 'submitted';
    return operation.printInfo.jobDisposition.isEqualToString(
      $.NSPrintCancelJob
    ) ? 'cancelled' : 'failed';
  } catch (error) {
    return 'failed';
  }
}
