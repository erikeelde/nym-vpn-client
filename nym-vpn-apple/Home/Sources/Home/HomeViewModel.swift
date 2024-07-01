import Combine
import SwiftUI
import AppSettings
import ConnectionManager
import CountriesManager
import Settings
import TunnelMixnet
import TunnelStatus
import Tunnels
import UIComponents
#if os(macOS)
import GRPCManager
import HelperManager
#endif

public class HomeViewModel: HomeFlowState {
    private let dateFormatter = DateComponentsFormatter()

    private var timer = Timer()
    private var cancellables = Set<AnyCancellable>()
    private var lastError: Error?
    @MainActor @Published private var activeTunnel: Tunnel?

    let title = "NymVPN".localizedString
    let connectToLocalizedTitle = "connectTo".localizedString
    let networkSelectLocalizedTitle = "selectNetwork".localizedString

    var appSettings: AppSettings
    var connectionManager: ConnectionManager
    var countriesManager: CountriesManager

#if os(macOS)
    var grpcManager: GRPCManager
    var helperManager: HelperManager
#endif
    var entryHopButtonViewModel = HopButtonViewModel(hopType: .entry)
    var exitHopButtonViewModel = HopButtonViewModel(hopType: .exit)
    var anonymousButtonViewModel = NetworkButtonViewModel(type: .mixnet5hop)
    var fastButtonViewModel = NetworkButtonViewModel(type: .mixnet2hop)

    // If no time connected is shown, should be set to empty string,
    // so the time connected label would not disappear and re-center other UI elements.
    @Published var timeConnected = " "
    @Published var statusButtonConfig = StatusButtonConfig.disconnected
    @Published var statusInfoState = StatusInfoState.initialising
    @Published var connectButtonState = ConnectButtonState.connect

#if os(iOS)
    public init(
        appSettings: AppSettings = AppSettings.shared,
        connectionManager: ConnectionManager = ConnectionManager.shared,
        countriesManager: CountriesManager = CountriesManager.shared
    ) {
        self.appSettings = appSettings
        self.connectionManager = connectionManager
        self.countriesManager = countriesManager
        super.init()

        setup()
    }
#endif
#if os(macOS)
    public init(
        appSettings: AppSettings = AppSettings.shared,
        connectionManager: ConnectionManager = ConnectionManager.shared,
        countriesManager: CountriesManager = CountriesManager.shared,
        grpcManager: GRPCManager = GRPCManager.shared,
        helperManager: HelperManager = HelperManager.shared
    ) {
        self.appSettings = appSettings
        self.connectionManager = connectionManager
        self.countriesManager = countriesManager
        self.grpcManager = grpcManager
        self.helperManager = helperManager
        super.init()

        setup()
    }
#endif
}

// MARK: - Navigation -

public extension HomeViewModel {
    func navigateToSettings() {
        path.append(HomeLink.settings)
    }

    func navigateToFirstHopSelection() {
        path.append(HomeLink.entryHop)
    }

    func navigateToLastHopSelection() {
        path.append(HomeLink.exitHop)
    }

    func navigateToAddCredentials() {
        path.append(HomeLink.settings)
        path.append(SettingsLink.addCredentials)
    }
}

// MARK: - Helpers -

public extension HomeViewModel {
    func shouldShowEntryHop() -> Bool {
        appSettings.isEntryLocationSelectionOn && !countriesManager.entryCountries.isEmpty
    }

    func configureConnectedTimeTimer() {
        timer = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: true) { [weak self] _ in
            self?.updateTimeConnected()
        }
    }

    func stopConnectedTimeTimerUpdates() {
        timer.invalidate()
    }
}

// MARK: - Connection -
public extension HomeViewModel {
    func connectDisconnect() {
        lastError = nil
        statusInfoState = .unknown
#if os(macOS)
        installHelperIfNeeded()
#endif
        guard appSettings.isCredentialImported
        else {
            navigateToAddCredentials()
            return
        }

        do {
            try connectionManager.connectDisconnect()
        } catch let error {
            statusInfoState = .error(message: error.localizedDescription)
        }
    }
}

// MARK: - Configuration -
private extension HomeViewModel {
    func setup() {
        setupDateFormatter()
        setupTunnelManagerObservers()
#if os(macOS)
        setupGRPCManagerObservers()
#endif
        setupCountriesManagerObservers()
    }

    func setupTunnelManagerObservers() {
        connectionManager.$isTunnelManagerLoaded.sink { [weak self] result in
            switch result {
            case .success, .none:
                self?.statusInfoState = .unknown
            case let .failure(error):
                self?.statusInfoState = .error(message: error.localizedDescription)
            }
        }
        .store(in: &cancellables)
#if os(iOS)
        connectionManager.$currentTunnel.sink { [weak self] tunnel in
            guard let tunnel, let self else { return }
            Task { @MainActor in
                self.activeTunnel = tunnel
            }
            self.configureTunnelStatusObservation(with: tunnel)
        }
        .store(in: &cancellables)
#endif
    }

    func setupDateFormatter() {
        dateFormatter.allowedUnits = [.hour, .minute, .second]
        dateFormatter.zeroFormattingBehavior = .pad
    }

    func setupCountriesManagerObservers() {
        countriesManager.$lastError.sink { [weak self] error in
            self?.lastError = error
        }
        .store(in: &cancellables)
    }

#if os(iOS)
    func configureTunnelStatusObservation(with tunnel: Tunnel) {
        tunnel.$status
            .debounce(for: .seconds(0.2), scheduler: DispatchQueue.global(qos: .background))
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] status in
                self?.updateUI(with: status)
            }
            .store(in: &cancellables)
    }
#endif

    func updateUI(with status: TunnelStatus) {
        Task { @MainActor in
            let newStatus: TunnelStatus
            if connectionManager.isReconnecting
                && (status == .disconnecting || status == .disconnected || status == .connecting) {
                newStatus = .reasserting
            } else {
                newStatus = status
            }
            statusButtonConfig = StatusButtonConfig(tunnelStatus: newStatus)
            connectButtonState = ConnectButtonState(tunnelStatus: newStatus)

            if let lastError {
                self.statusInfoState = .error(message: lastError.localizedDescription)
            } else {
                statusInfoState = StatusInfoState(tunnelStatus: newStatus)
            }
#if os(macOS)
            updateConnectedStartDateMacOS(with: status)
#endif
        }
    }

    func fetchCountries() {
        countriesManager.fetchCountries()
    }
}

// MARK: - iOS -
#if os(iOS)
private extension HomeViewModel {
    func updateTimeConnected() {
        Task { @MainActor in
            let emptyTimeText = " "
            guard let activeTunnel,
                  activeTunnel.status == .connected,
                  let connectedDate = activeTunnel.tunnel.connection.connectedDate
            else {
                guard timeConnected != emptyTimeText else { return }
                timeConnected = emptyTimeText
                return
            }
            timeConnected = dateFormatter.string(from: connectedDate, to: Date()) ?? emptyTimeText
        }
    }
}
#endif

// MARK: - macOS -
#if os(macOS)
private extension HomeViewModel {
    func setupGRPCManagerObservers() {
        grpcManager.$lastError.sink { [weak self] error in
            self?.lastError = error
        }
        .store(in: &cancellables)

        grpcManager.$tunnelStatus
            .debounce(for: .seconds(0.15), scheduler: DispatchQueue.global(qos: .background))
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] status in
                self?.updateUI(with: status)
            }
            .store(in: &cancellables)
    }

    func installHelperIfNeeded() {
        // TODO: check if possible to split is helper running vs isHelperAuthorized
        guard helperManager.isHelperRunning() && helperManager.isHelperAuthorized()
        else {
            do {
                _ = try helperManager.authorizeAndInstallHelper()
            } catch let error {
                statusInfoState = .error(message: error.localizedDescription)
            }
            return
        }
    }

    func updateConnectedStartDateMacOS(with status: TunnelStatus) {
        guard status == .connected else { return }
        grpcManager.status()
    }

    func updateTimeConnected() {
        Task { @MainActor in
            let emptyTimeText = " "
            guard grpcManager.tunnelStatus == .connected,
                  let connectedDate = grpcManager.connectedDate
            else {
                guard timeConnected != emptyTimeText else { return }
                timeConnected = emptyTimeText
                return
            }
            timeConnected = dateFormatter.string(from: connectedDate, to: Date()) ?? emptyTimeText
        }
    }
}
#endif
