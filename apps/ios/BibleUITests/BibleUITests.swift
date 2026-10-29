import XCTest

/// The main flows, driven through the interface. Each test starts the app with no saved data
/// (`--uitest`), so it opens at John 1 in the KJV.
final class BibleUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["--uitest"]
        app.launch()
        XCTAssertTrue(reference.waitForExistence(timeout: 10))
    }

    private var reference: XCUIElement { app.buttons["reference"] }

    private func verse(_ id: String) -> XCUIElement { app.staticTexts[id] }

    /// Waits for the toolbar to name a chapter.
    private func expectChapter(_ title: String, file: StaticString = #filePath, line: UInt = #line) {
        let shown = NSPredicate(format: "label BEGINSWITH %@", title + ".")
        let found = XCTNSPredicateExpectation(predicate: shown, object: reference)
        XCTAssertEqual(XCTWaiter.wait(for: [found], timeout: 5), .completed, "expected \(title), saw \(reference.label)", file: file, line: line)
    }

    func testOpensAtJohnOne() {
        expectChapter("John 1")
        XCTAssertTrue(verse("John 1:1").exists)
        XCTAssertTrue(verse("John 1:1").label.contains("In the beginning was the Word"))
    }

    func testScrollingReadsOnIntoTheNextChapters() {
        for _ in 0..<12 { app.swipeUp(velocity: .fast) }
        let title = reference.label
        XCTAssertFalse(title.hasPrefix("John 1."), "still at \(title)")
        XCTAssertTrue(title.hasPrefix("John"), "left John: \(title)")
        // And back, across the chapter boundary into Luke.
        for _ in 0..<24 { app.swipeDown(velocity: .fast) }
        let shown = NSPredicate(format: "label BEGINSWITH 'Luke'")
        let back = XCTNSPredicateExpectation(predicate: shown, object: reference)
        XCTAssertEqual(XCTWaiter.wait(for: [back], timeout: 5), .completed, "saw \(reference.label)")
    }

    func testJumpsToATypedReference() {
        reference.tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.tap()
        field.typeText("rom 8:28\n")
        expectChapter("Romans 8")
        XCTAssertTrue(verse("Romans 8:28").waitForExistence(timeout: 3))
        XCTAssertTrue(verse("Romans 8:28").isHittable)
    }

    func testRotatingKeepsYourPlace() {
        addTeardownBlock { XCUIDevice.shared.orientation = .portrait }
        reference.tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.tap()
        field.typeText("rom 8:28\n")
        expectChapter("Romans 8")
        XCTAssertTrue(verse("Romans 8:28").waitForExistence(timeout: 3))
        for orientation in [UIDeviceOrientation.landscapeLeft, .portrait] {
            XCUIDevice.shared.orientation = orientation
            sleep(1)
            expectChapter("Romans 8")
            XCTAssertTrue(verse("Romans 8:28").isHittable, "Romans 8:28 left the screen at orientation \(orientation.rawValue)")
        }
    }

    func testRotatingKeepsJudeChapterStart() {
        expectRotationKeepsChapterStart("jude 1", title: "Jude 1")
    }

    func testRotatingKeepsRomansChapterStart() {
        expectRotationKeepsChapterStart("rom 8", title: "Romans 8")
    }

    private func expectRotationKeepsChapterStart(_ query: String, title: String) {
        addTeardownBlock { XCUIDevice.shared.orientation = .portrait }
        reference.tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.tap()
        field.typeText(query + "\n")
        expectChapter(title)
        for orientation in [UIDeviceOrientation.landscapeLeft, .portrait] {
            XCUIDevice.shared.orientation = orientation
            sleep(1)
            let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            shot.name = "\(title)-orientation-\(orientation.rawValue)"
            shot.lifetime = .keepAlways
            add(shot)
            expectChapter(title)
            XCTAssertTrue(verse("\(title):1").isHittable, "Chapter start left the screen")
        }
    }

    func testReferenceRemainsUsableAtAccessibilityTextSize() {
        for (size, category) in [
            ("Default", "UICTContentSizeCategoryL"),
            ("AX-XL", "UICTContentSizeCategoryAccessibilityXL"),
            ("AX-XXXL", "UICTContentSizeCategoryAccessibilityXXXL"),
        ] {
            app.terminate()
            app.launchArguments = ["--uitest", "-UIPreferredContentSizeCategoryName", category]
            app.launch()
            XCTAssertTrue(reference.waitForExistence(timeout: 10))
            for (query, title, short) in [
                ("luke 24", "Luke 24", "Luke 24"),
                ("song 8", "Song of Solomon 8", "Song 8"),
                ("1 thess 5", "1 Thessalonians 5", "1 Thess 5"),
                ("lam 5", "Lamentations 5", "Lam 5"),
            ] {
                reference.tap()
                let field = app.searchFields.firstMatch
                XCTAssertTrue(field.waitForExistence(timeout: 3))
                field.tap()
                field.typeText(query + "\n")
                expectChapter(title)
                // The value comes from the chosen ViewThatFits label, not the full VoiceOver label.
                let visible = reference.value as? String
                let font = UIFont.preferredFont(forTextStyle: .headline, compatibleWith: UITraitCollection(
                    preferredContentSizeCategory: size == "Default" ? .large : .extraExtraLarge))
                let fullWidth = (title as NSString).size(withAttributes: [.font: font]).width
                let shortWidth = (short as NSString).size(withAttributes: [.font: font]).width
                let measurement = XCTAttachment(string: "\(size), \(title), visible=\(visible ?? "nil"), fullWidth=\(fullWidth), shortWidth=\(shortWidth), reference=\(reference.frame), translation=\(app.buttons["translation"].frame), search=\(app.buttons["search"].frame), app=\(app.frame)")
                measurement.name = "Toolbar measurement-\(size)-\(title)"
                measurement.lifetime = .keepAlways
                add(measurement)
                let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
                shot.name = "Toolbar-\(size)-\(title)"
                shot.lifetime = .keepAlways
                add(shot)
                XCTAssertTrue(visible == title || visible == short, "Unexpected visible reference: \(visible ?? "nil")")
                if query == "luke 24" || (size == "Default" && app.frame.width >= 402 && query == "lam 5") {
                    XCTAssertEqual(visible, title, "Use the full reference when it fits at \(size)")
                }
                if app.frame.width <= 375 && size != "Default" && query != "luke 24" {
                    XCTAssertEqual(visible, short, "Use a readable abbreviation at \(size)")
                }
                let labelWidth = visible == title ? fullWidth : shortWidth
                XCTAssertEqual(reference.frame.width, max(44, labelWidth + 26), accuracy: 1,
                               "Size the reference pill to its visible label")
                XCTAssertEqual(reference.frame.height, 44, accuracy: 1)
                XCTAssertGreaterThanOrEqual(reference.frame.width, 44)
                XCTAssertLessThanOrEqual(reference.frame.maxX, app.buttons["translation"].frame.minX)
                XCTAssertLessThanOrEqual(app.buttons["translation"].frame.maxX, app.buttons["search"].frame.minX)
                for id in ["reference", "translation", "search", "settings", "study"] {
                    XCTAssertTrue(app.buttons[id].isHittable, id)
                }
            }
        }
    }

    func testPicksABookAndChapter() {
        reference.tap()
        let romans = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Romans'")).firstMatch
        XCTAssertTrue(romans.waitForExistence(timeout: 3))
        romans.tap()
        let twelve = app.buttons["Romans 12"]
        XCTAssertTrue(twelve.waitForExistence(timeout: 3))
        twelve.tap()
        expectChapter("Romans 12")
        XCTAssertTrue(verse("Romans 12:1").label.contains("living sacrifice"))
    }

    func testPsalmsOpenFromAReference() {
        reference.tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.tap()
        field.typeText("ps 23\n")
        expectChapter("Psalm 23")
        XCTAssertTrue(verse("Psalm 23:1").waitForExistence(timeout: 3))
        XCTAssertTrue(verse("Psalm 23:1").label.contains("shepherd"))
    }

    func testSwitchesTranslation() {
        XCTAssertTrue(verse("John 1:5").label.contains("shineth in darkness"))
        app.buttons["translation"].tap()
        let web = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'WEB'")).firstMatch
        XCTAssertTrue(web.waitForExistence(timeout: 3))
        web.tap()
        let switched = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "label CONTAINS 'shines in the darkness'"), object: verse("John 1:5"))
        XCTAssertEqual(XCTWaiter.wait(for: [switched], timeout: 5), .completed, verse("John 1:5").label)
        expectChapter("John 1")
    }

    func testReopensWhereYouLeftOff() {
        reference.tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.tap()
        field.typeText("rom 8:28\n")
        expectChapter("Romans 8")
        app.buttons["translation"].tap()
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'BSB'")).firstMatch.tap()

        XCUIDevice.shared.press(.home)
        app.terminate()
        app.launchArguments = ["--uitest", "--keep-data"]
        app.launch()
        XCTAssertTrue(reference.waitForExistence(timeout: 10))
        expectChapter("Romans 8")
        XCTAssertTrue(verse("Romans 8:28").isHittable)
        XCTAssertTrue(app.buttons["translation"].label.contains("Berean"), app.buttons["translation"].label)
    }

    func testSearchFindsAndOpensAVerse() {
        app.buttons["search"].tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.typeText("love one another")
        let hit = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'John 13:34'")).firstMatch
        XCTAssertTrue(hit.waitForExistence(timeout: 5))
        hit.tap()
        expectChapter("John 13")
        XCTAssertTrue(verse("John 13:34").isHittable)
    }

    func testSearchLoadsMoreAsYouScroll() {
        app.buttons["search"].tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.typeText("lamb\n")
        XCTAssertTrue(row("Genesis 21:28").waitForExistence(timeout: 5))
        // The last of 175 hits, on the fourth page.
        let last = row("Revelation 22:3")
        for _ in 0..<40 where !last.exists { app.swipeUp(velocity: .fast) }
        XCTAssertTrue(last.exists)
    }

    func testMisspelledSearchOffersASuggestion() {
        app.buttons["search"].tap()
        let field = app.searchFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 3))
        field.typeText("bethsaida pool")
        let suggestion = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Search “bethesda pool”'")).firstMatch
        XCTAssertTrue(suggestion.waitForExistence(timeout: 5))
        suggestion.tap()
        XCTAssertTrue(
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'John 5:2'")).firstMatch.waitForExistence(timeout: 5))
    }

    func testHighlightBookmarkAndNoteShowInStudy() {
        let first = verse("John 1:1")
        first.tap()
        app.selectionAction("Highlight Green").tap()

        first.tap()
        app.selectionAction("Bookmark").tap()

        verse("John 1:3").tap()
        app.selectionAction("Note").tap()
        let note = app.textFields["Write a note"].exists ? app.textFields["Write a note"] : app.textViews.firstMatch
        XCTAssertTrue(note.waitForExistence(timeout: 3))
        note.typeText("All things, without exception.")
        app.buttons["Save"].tap()

        app.buttons["study"].tap()
        XCTAssertTrue(row("John 1:1").waitForExistence(timeout: 3), "bookmark listed")
        app.buttons["Highlights"].tap()
        XCTAssertTrue(row("John 1:1").waitForExistence(timeout: 3), "highlight listed")
        app.buttons["Notes"].tap()
        XCTAssertTrue(row("John 1:3").waitForExistence(timeout: 3), "note listed")
        XCTAssertTrue(row("John 1:3").label.contains("without exception"))
    }

    func testCopiesAVerseWithItsReference() {
        verse("John 1:4").tap()
        app.selectionAction("Copy").tap()
        XCTAssertTrue(app.staticTexts["Copied John 1:4"].waitForExistence(timeout: 3))
    }

    func testThemeAndTextSizeApplyAtOnce() {
        app.buttons["settings"].tap()
        app.buttons["Dark theme"].tap()
        let slider = app.sliders["Text size"]
        XCTAssertTrue(slider.waitForExistence(timeout: 3))
        slider.adjust(toNormalizedSliderPosition: 0.9)
        app.buttons["Done"].tap()
        expectChapter("John 1")
        XCTAssertTrue(verse("John 1:1").exists)
    }

    func testScrollingDoesNotHitch() {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        measure(metrics: [XCTOSSignpostMetric.scrollDecelerationMetric], options: options) {
            app.swipeUp(velocity: .fast)
        }
    }

    private func row(_ reference: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", reference)).firstMatch
    }
}

extension XCUIApplication {
    /// A button in the selection bar, once the bar has slid into place (0.28 s): a tap while it
    /// moves lands on the text behind it.
    func selectionAction(_ label: String) -> XCUIElement {
        let button = buttons[label]
        XCTAssertTrue(button.waitForExistence(timeout: 3), label)
        usleep(400_000)
        return button
    }
}
