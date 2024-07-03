#if os(iOS)
import UIKit
#endif

#if os(macOS)
import AppKit
#endif

import Constants

public final class ExternalLinkManager {
    public static let shared = ExternalLinkManager()

#if os(iOS)
    public func openExternalURL(urlString: String?) throws {
        guard let urlString, let url = URL(string: urlString)
        else {
            throw GeneralNymError.invalidUrl
        }
        UIApplication.shared.open(url)
    }
#endif

#if os(macOS)
    public func openExternalURL(urlString: String?) throws {
        guard let urlString, let url = URL(string: urlString)
        else {
            throw GeneralNymError.invalidUrl
        }
        NSWorkspace.shared.open(url)
    }
#endif
}