import AppKit
import CryptoKit
import Darwin
import SwiftUI

private let panelSizeChanged = Notification.Name("NovaPanelSize")
private let panelHidden = Notification.Name("NovaPanelHide")
final class NovaFloatingPanel: NSPanel {
  override var canBecomeKey: Bool { true }
  override var canBecomeMain: Bool { false }
}
@MainActor final class IslandDelegate: NSObject, NSApplicationDelegate {
  let model = NovaModel()
  var panel: NovaFloatingPanel?
  var status: NSStatusItem?
  var lockDescriptor: Int32 = -1
  var showNotification: Notification.Name?
  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
    let defaultRoot = FileManager.default.urls(
      for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Nova")
    let root = URL(fileURLWithPath: Bridge.directory ?? defaultRoot.path).resolvingSymlinksInPath()
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let digest = SHA256.hash(data: Data(root.path.utf8)).map { String(format: "%02x", $0) }.joined()
    let notification = Notification.Name("dev.nova.panel." + digest)
    lockDescriptor = Darwin.open(
      root.appendingPathComponent("desktop-ui.lock").path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
    if lockDescriptor >= 0 && flock(lockDescriptor, LOCK_EX | LOCK_NB) != 0 {
      DistributedNotificationCenter.default().postNotificationName(
        notification, object: nil, userInfo: nil, deliverImmediately: true)
      NSApp.terminate(nil)
      return
    }
    showNotification = notification
    DistributedNotificationCenter.default().addObserver(
      self, selector: #selector(showPanel), name: notification, object: nil)

    let panel = NovaFloatingPanel(
      contentRect: NSRect(x: 0, y: 0, width: 210, height: 44),
      styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    panel.level = .floating
    panel.isFloatingPanel = true
    panel.hidesOnDeactivate = false
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
    panel.appearance = NSAppearance(named: .darkAqua)
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = true
    panel.isMovableByWindowBackground = true
    panel.title = "Nova"
    panel.contentView = NSHostingView(rootView: Dashboard().environmentObject(model))
    self.panel = panel
    NotificationCenter.default.addObserver(
      self, selector: #selector(resize(_:)), name: panelSizeChanged, object: nil)
    NotificationCenter.default.addObserver(
      self, selector: #selector(hidePanel), name: panelHidden, object: nil)
    NotificationCenter.default.addObserver(
      self, selector: #selector(reposition),
      name: NSApplication.didChangeScreenParametersNotification, object: nil)
    let status = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    if let button = status.button {
      let image = NSImage(contentsOf: Bundle.main.resourceURL!.appendingPathComponent("Nova.icns"))
      image?.size = NSSize(width: 19, height: 19)
      button.image = image
      button.toolTip = "Nova — show or hide panel"
      button.target = self
      button.action = #selector(togglePanel)
    }
    self.status = status
    reposition()
    panel.makeKeyAndOrderFront(nil)
  }
  @objc func reposition() {
    guard let panel, let screen = NSScreen.main ?? NSScreen.screens.first else { return }
    let top =
      screen.frame.maxY
      - max(screen.safeAreaInsets.top, screen.frame.maxY - screen.visibleFrame.maxY) - 8
    panel.setFrameOrigin(
      NSPoint(x: screen.frame.midX - panel.frame.width / 2, y: top - panel.frame.height))
  }
  @objc func resize(_ notification: Notification) {
    guard let panel else { return }
    let expanded = notification.userInfo?["expanded"] as? Bool ?? true
    let size = expanded ? NSSize(width: 360, height: 340) : NSSize(width: 210, height: 44)
    let top = panel.frame.maxY
    let center = panel.frame.midX
    panel.setFrame(
      NSRect(
        x: center - size.width / 2, y: top - size.height, width: size.width, height: size.height),
      display: true, animate: true)
  }
  @objc func showPanel() {
    reposition()
    panel?.makeKeyAndOrderFront(nil)
  }
  func applicationWillTerminate(_ notification: Notification) {
    if lockDescriptor >= 0 { Darwin.close(lockDescriptor) }
  }
  @objc func hidePanel() { panel?.orderOut(nil) }
  @objc func togglePanel() {
    guard let panel else { return }
    if panel.isVisible {
      panel.orderOut(nil)
    } else {
      reposition()
      panel.makeKeyAndOrderFront(nil)
    }
  }
  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool
  {
    reposition()
    panel?.makeKeyAndOrderFront(nil)
    return true
  }
}
@main struct NovaDesktop: App {
  @NSApplicationDelegateAdaptor(IslandDelegate.self) var delegate
  var body: some Scene { SwiftUI.Settings { EmptyView() } }
}
struct NovaGlyph: Shape {
  func path(in rect: CGRect) -> Path {
    var p = Path()
    p.move(to: CGPoint(x: rect.width * 0.25, y: rect.height * 0.75))
    p.addLine(to: CGPoint(x: rect.width * 0.25, y: rect.height * 0.25))
    p.addLine(to: CGPoint(x: rect.width * 0.70, y: rect.height * 0.75))
    p.addLine(to: CGPoint(x: rect.width * 0.70, y: rect.height * 0.25))
    return p
  }
}
struct NovaLogo: View {
  var body: some View {
    ZStack {
      RoundedRectangle(cornerRadius: 7).fill(
        LinearGradient(
          colors: [accent, Color(red: 0.98, green: 0.48, blue: 0.23)], startPoint: .bottomLeading,
          endPoint: .topTrailing))
      NovaGlyph().stroke(
        .white, style: StrokeStyle(lineWidth: 2.5, lineCap: .round, lineJoin: .round)
      ).padding(3)
      Circle().fill(.white).frame(width: 3, height: 3).offset(x: 8, y: -8)
    }.frame(width: 28, height: 28).accessibilityLabel("Nova logo")
  }
}
struct Dashboard: View {
  @EnvironmentObject var model: NovaModel
  @State var expanded = false
  @State var tab = "Work"
  @State var projectForm = false
  @State var planForm = false
  @State var goalForm = false
  @State var providerForm = false
  @State var editingProviderKey: ProviderItem?
  @State var removingProvider: ProviderItem?
  @State var routesForm = false
  @State var details = false
  @State var executionReview = false
  @State var approveReview = false
  @State var trustProject: ProjectItem?
  @State var removingProject: ProjectItem?
  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 9) {
        NovaLogo()
        Text("nova").font(.system(size: 16, weight: .semibold, design: .rounded))
        Circle().fill(model.connected ? Color.green : .orange).frame(width: 5, height: 5).help(
          model.connected ? "Local service connected" : "Service unavailable")
        if !expanded { Text(pillActivity).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
        Spacer(minLength: 0)
        if model.busy { ProgressView().controlSize(.mini) }
        Button {
          expanded.toggle()
          NotificationCenter.default.post(
            name: panelSizeChanged, object: nil, userInfo: ["expanded": expanded])
        } label: {
          Image(systemName: expanded ? "chevron.up" : "chevron.down").font(
            .system(size: 10, weight: .bold))
        }.buttonStyle(.plain).help(expanded ? "Collapse Nova" : "Expand Nova")
        if expanded {
          Menu {
            Button("Refresh") { Task { await model.refresh() } }
            Button("Hide panel") { NotificationCenter.default.post(name: panelHidden, object: nil) }
            Divider()
            Button("Quit panel") { NSApp.terminate(nil) }
          } label: {
            Image(systemName: "ellipsis").font(.system(size: 12, weight: .bold))
          }.menuStyle(.borderlessButton).fixedSize().help("Panel options")
        }
      }.padding(.horizontal, 13).frame(height: expanded ? 48 : 44)
      if expanded {
        Picker("Section", selection: $tab) {
          Text("Work").tag("Work")
          Text("Projects").tag("Projects")
          Text("Models").tag("Models")
        }.pickerStyle(.segmented).labelsHidden().padding(.horizontal, 12).padding(.bottom, 8)
        if let error = model.error {
          HStack(alignment: .top) {
            Text(error).font(.caption).foregroundStyle(.orange).lineLimit(3)
            Spacer(minLength: 0)
            Button {
              model.error = nil
            } label: {
              Image(systemName: "xmark").font(.caption2)
            }
          }.buttonStyle(.plain).padding(.horizontal, 13).padding(.bottom, 6)
        }
        if let notice = model.notice {
          HStack {
            Text(notice).font(.caption).foregroundStyle(.green).lineLimit(2)
            Spacer(minLength: 0)
            Button {
              model.notice = nil
            } label: {
              Image(systemName: "xmark").font(.caption2)
            }
          }.buttonStyle(.plain).padding(.horizontal, 13).padding(.bottom, 6)
        }
        ScrollView {
          VStack(alignment: .leading, spacing: 10) {
            switch tab {
            case "Projects": projects
            case "Models": models
            default: work
            }
          }.padding(.horizontal, 13).padding(.bottom, 10).frame(
            maxWidth: .infinity, alignment: .leading)
        }
        Divider().opacity(0.4)
        HStack {
          Text(activity).font(.system(size: 10)).foregroundStyle(.secondary)
          Spacer()
          Button("Manual plan") { planForm = true }.controlSize(.small).disabled(
            model.projects.isEmpty || model.busy)
          Button("New goal", systemImage: "sparkles") { goalForm = true }.buttonStyle(
            .borderedProminent
          ).controlSize(.small).disabled(model.projects.isEmpty || model.busy)
        }.padding(.horizontal, 13).frame(height: 40)
      }
    }.frame(width: expanded ? 360 : 210, height: expanded ? 340 : 44).background(
      Color.black.opacity(0.94), in: RoundedRectangle(cornerRadius: 19)
    ).overlay(RoundedRectangle(cornerRadius: 19).stroke(.white.opacity(0.14))).tint(accent)
      .preferredColorScheme(.dark)
      .task {
        do { _ = try await Bridge.call(["op": "start"]) } catch {
          model.error = error.localizedDescription
        }
        await model.refresh()
      }
      .task {
        while !Task.isCancelled {
          try? await Task.sleep(nanoseconds: 4_000_000_000)
          if !model.busy { await model.refresh() }
        }
      }
      .sheet(isPresented: $projectForm) { ProjectForm().environmentObject(model) }
      .sheet(isPresented: $planForm) { PlanForm().environmentObject(model) }
      .sheet(isPresented: $goalForm) { GoalForm().environmentObject(model) }
      .sheet(isPresented: $providerForm) { ProviderForm().environmentObject(model) }
      .sheet(item: $editingProviderKey) { provider in
        ProviderKeyForm(provider: provider).environmentObject(model)
      }
      .sheet(isPresented: $executionReview) { ExecuteReview().environmentObject(model) }
      .sheet(isPresented: $routesForm) { RoutesSheet().environmentObject(model) }
      .sheet(isPresented: $details) { CompactRunDetails().environmentObject(model) }
      .confirmationDialog(
        "Remove \(removingProvider?.id ?? "provider")?",
        isPresented: Binding(
          get: { removingProvider != nil },
          set: { if !$0 { removingProvider = nil } }),
        titleVisibility: .visible
      ) {
        Button("Remove connection and saved key", role: .destructive) {
          guard let provider = removingProvider else { return }
          removingProvider = nil
          model.perform {
            _ = try await Bridge.call(["op": "remove", "name": provider.id])
            model.notice = "Provider removed."
          }
        }
      } message: {
        Text("Its models and routes will be removed. Running executions retain their current configuration.")
      }
      .confirmationDialog(
        "Approve this plan?", isPresented: $approveReview, titleVisibility: .visible
      ) {
        Button("Approve reviewed plan") {
          guard let run = model.selectedRun, let plan = model.plan else { return }
          model.perform {
            _ = try await Bridge.api("POST", "/v1/plans/\(run.id)/\(plan.version)/approve")
            model.notice = "Approved. Choose Execute when ready."
          }
        }
      } message: {
        Text(
          model.plan?.requires_attention?.isEmpty == false
            ? "Package scripts or make targets need attention in Details. Review their exact commands before approval. Approval does not start workers."
            : "Review task scope, acceptance commands and budgets in Details first. Approval does not start workers."
        )
      }
      .confirmationDialog(
        "Enable local execution?",
        isPresented: Binding(get: { trustProject != nil }, set: { if !$0 { trustProject = nil } }),
        titleVisibility: .visible
      ) {
        Button("Trust this repository") {
          guard let project = trustProject else { return }
          trustProject = nil
          model.perform {
            _ = try await Bridge.api(
              "POST", "/v1/projects/\(project.id)/trust", ["trusted_local": true])
            model.notice = "Local execution enabled. Create and approve a task plan."
          }
        }
      } message: {
        Text(
          "\(trustProject?.repo_path ?? "")\nRepository code and tests run with your user permissions. An approved plan is still required. Snapshot mode leaves unfinished files untouched."
        )
      }
      .confirmationDialog(
        "Remove empty project?",
        isPresented: Binding(
          get: { removingProject != nil }, set: { if !$0 { removingProject = nil } }),
        titleVisibility: .visible
      ) {
        Button("Remove registration", role: .destructive) {
          guard let project = removingProject else { return }
          removingProject = nil
          model.perform {
            _ = try await Bridge.api("DELETE", "/v1/projects/\(project.id)")
            model.notice = "Project registration removed. Files are untouched."
          }
        }
      } message: {
        Text("Only projects without runs can be removed. Files on disk are kept.")
      }
  }
  var pillActivity: String {
    if model.busy { return "Working" }
    if !model.connected { return "Offline" }
    if model.planning?.status == "planning" { return "Planning" }
    if model.selectedRun?.state == "awaiting_approval" { return "Review" }
    let active = model.tasks.filter {
      ["running", "leased", "verifying", "integrating"].contains($0.state)
    }.count
    return active > 0 ? "\(active) active" : "Ready"
  }
  var activity: String {
    if model.busy { return "Working…" }
    if !model.connected { return "Connecting…" }
    if model.planning?.status == "planning" { return "Main agent planning…" }
    if model.selectedRun?.state == "awaiting_approval" { return "Approval needed" }
    if model.projects.isEmpty { return "Add a project" }

    let active = model.tasks.filter {
      ["running", "leased", "verifying", "integrating"].contains($0.state)
    }.count
    return active > 0
      ? "\(active) agents active" : model.connected ? "Ready · local" : "Connecting…"
  }
  var work: some View {
    VStack(alignment: .leading, spacing: 10) {
      if model.runs.isEmpty {
        HStack {
          Image(systemName: "sparkles").foregroundStyle(accent)
          Text("Ready when you are").font(.headline)
        }.padding(.top, 6)
        Text(
          model.projects.isEmpty
            ? "Add a repository, then create a task."
            : "Describe a goal. Nova plans subtasks; you approve, then execute."
        ).font(.caption).foregroundStyle(.secondary)
        if model.projects.isEmpty {
          Button("Add project", systemImage: "folder.badge.plus") { projectForm = true }
            .buttonStyle(.bordered).controlSize(.small)
        } else if let project = model.projects.first, !project.trusted_local {
          Text("\(project.name) needs your trust before execution.").font(.caption).foregroundStyle(
            .secondary)
          Button("Enable local execution") { trustProject = project }.controlSize(.small)
        }
      } else {
        Picker(
          "Run",
          selection: Binding(
            get: { model.selectedRun?.id ?? "" },
            set: { id in if let run = model.runs.first(where: { $0.id == id }) { model.select(run) }
            })
        ) {
          Text("Choose a run").tag("")
          ForEach(model.runs) { run in
            Text(
              "\(model.projects.first(where:{$0.id==run.project_id})?.name ?? String(run.id.prefix(8))) · \(run.state == "running" ? "approved" : run.state)"
            ).tag(run.id)
          }
        }.labelsHidden()
        if let run = model.selectedRun {
          HStack {
            Text(model.plan?.plan.idea ?? "Task run").font(.headline).lineLimit(2)
            Spacer(minLength: 0)
            StatePill(
              state: run.state == "running" && model.tasks.allSatisfy { $0.attempt == 0 }
                ? "ready" : run.state)
          }
          if let planning = model.planning {
            Text(planning.summary).font(.caption).foregroundStyle(
              planning.status == "failed" ? .orange : .secondary
            ).lineLimit(3)
          }
          if !model.tasks.isEmpty {
            ProgressView(
              value: Double(model.tasks.filter { $0.state == "completed" }.count),
              total: Double(model.tasks.count))
            ForEach(model.tasks.prefix(3)) { task in
              HStack {
                Circle().fill(stateColor(task.state)).frame(width: 5, height: 5)
                Text(task.spec.title).font(.caption).lineLimit(1)
                Spacer()
                Text(task.state).font(.system(size: 10)).foregroundStyle(.secondary)
              }
            }
          }
          HStack {
            Button("Details") { details = true }
            if run.state == "awaiting_approval" {
              Button("Approve plan") { approveReview = true }.buttonStyle(.borderedProminent)
            }
            if run.state == "running" {
              if let project = model.projects.first(where: { $0.id == run.project_id }),
                !project.trusted_local
              {
                Button("Enable execution") { trustProject = project }.buttonStyle(
                  .borderedProminent)
              } else {
                Button("Execute…") { executionReview = true }.buttonStyle(.borderedProminent)
              }
              Button {
                model.control("pause")
              } label: {
                Image(systemName: "pause.fill")
              }.help("Pause new assignments")
            }
            if run.state == "paused" { Button("Resume") { model.control("resume") } }
          }.controlSize(.small)
        }
      }
    }
  }
  var projects: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack {
        Text("Projects").font(.headline)
        Spacer()
        Button {
          projectForm = true
        } label: {
          Image(systemName: "plus")
        }.help("Add project")
      }
      if model.projects.isEmpty {
        Text("No repository added yet.").font(.caption).foregroundStyle(.secondary)
      }
      ForEach(model.projects) { project in
        VStack(alignment: .leading, spacing: 5) {
          HStack {
            Text(project.name).font(.callout.weight(.medium)).lineLimit(1)
            Spacer()
            Menu {
              if !project.trusted_local {
                Button("Enable local execution") { trustProject = project }
              }
              Button("Remove empty project", role: .destructive) { removingProject = project }
            } label: {
              Image(systemName: "ellipsis")
            }.menuStyle(.borderlessButton).fixedSize()
          }
          Text(project.repo_path).font(.system(size: 10, design: .monospaced)).lineLimit(1)
            .truncationMode(.middle).foregroundStyle(.secondary)
          HStack {
            Text(project.trusted_local ? "Local execution enabled" : "Trust needed").font(.caption2)
              .foregroundStyle(project.trusted_local ? .green : .orange)
            Spacer()
            if !project.trusted_local {
              Button("Enable") { trustProject = project }.controlSize(.mini)
            }
          }
          Divider().padding(.top, 3)
        }
      }
    }
  }
  var models: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack {
        Text("Model accounts").font(.headline)
        Spacer()
        Button {
          providerForm = true
        } label: {
          Image(systemName: "plus")
        }.help("Connect provider")
      }
      if model.settings?.providers.isEmpty ?? true {
        Text("Connect OpenAI, Claude or a compatible endpoint.").font(.caption).foregroundStyle(
          .secondary)
        Button("Connect provider") { providerForm = true }.controlSize(.small)
      }
      ForEach(model.settings?.providers ?? []) { provider in
        HStack {
          Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
          Text(provider.id).font(.callout)
          Spacer()
          Text(provider.kind).font(.caption).foregroundStyle(.secondary)
          Menu {
            if provider.key_editable {
              Button("Update API key…") { editingProviderKey = provider }
            }
            Button("Remove provider…", role: .destructive) { removingProvider = provider }
          } label: {
            Image(systemName: "ellipsis")
          }.menuStyle(.borderlessButton).fixedSize().help("Manage \(provider.id)")
        }
      }
      Text("\(model.settings?.models.filter{$0.ready}.count ?? 0) ready models").font(.caption)
        .foregroundStyle(.secondary)
      Button("Model routes…", systemImage: "arrow.triangle.branch") { routesForm = true }
        .controlSize(.small)
    }
  }
}
struct RoutesSheet: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text("Model routes").font(.headline)
        Spacer()
        Button("Done") { dismiss() }
      }
      ScrollView {
        VStack(spacing: 8) {
          ForEach(useCases, id: \.self) { role in RouteRow(role: role).environmentObject(model) }
        }
      }
    }.padding(18).frame(width: 440, height: 500)
  }
}
struct CompactRunDetails: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text("Run details").font(.headline)
        Spacer()
        Button("Done") { dismiss() }
      }
      ScrollView {
        VStack(alignment: .leading, spacing: 14) {
          if let plan = model.plan {
            Text(plan.plan.idea).font(.headline)
            ForEach(plan.plan.tasks, id: \.id) { task in
              VStack(alignment: .leading, spacing: 5) {
                Text(task.title).font(.callout.weight(.semibold))
                if plan.requires_attention?.contains(task.id) == true {
                  Text("Review this package script or make target before approval.")
                    .font(.caption).foregroundStyle(.orange)
                }
                Text(task.description).font(.caption).foregroundStyle(.secondary)
                Text(task.candidate_paths.joined(separator: "\n")).font(
                  .system(.caption, design: .monospaced))
                ForEach(Array(task.acceptance.enumerated()), id: \.offset) { item in
                  Text(
                    item.element.command.map { argvText($0) } ?? item.element.description
                      ?? "Assertion"
                  ).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                }
                Text(String(format: "Spend cap $%.2f", task.limits.cost_cap_usd)).font(.caption2)
                Divider()
              }
            }
          }
          ForEach(model.tasks) { task in
            Button("Evidence: \(task.spec.title)") { model.inspect(task) }
          }
          if let evidence = model.evidence {
            if let failure = evidence.failure {
              Text("Worker blocked: \(failure)").font(.caption).foregroundStyle(.orange)
                .textSelection(.enabled)
            }
            if let commit = evidence.commit {
              Text(
                "Task: \(commit.task_commit_sha)\nIntegrated: \(commit.integration_commit_sha ?? "pending")"
              ).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            }
            ForEach(evidence.tests) { test in
              Text(
                "\(test.exit_code == 0 ? "Passed" : "Failed / unknown") · \(argvText(test.command))"
              ).font(.caption)
            }
          }
          DisclosureGroup("Source revision") {
            Text(model.sourceDescription).font(.system(.caption2, design: .monospaced))
              .textSelection(.enabled)
          }
          DisclosureGroup("Usage") {
            Text(model.usage).font(.system(.caption2, design: .monospaced)).textSelection(.enabled)
          }
          DisclosureGroup("Activity") {
            ForEach(model.events.suffix(30).reversed()) { event in
              Text(
                "#\(event.sequence) \(event.task_id ?? "Run"): \(event.to_state ?? event.kind ?? "event")"
              ).font(.caption2)
            }
          }
          ForEach(model.approvals) { approval in
            VStack(alignment: .leading, spacing: 5) {
              Text("Approval · \(approval.state)").font(.caption.weight(.semibold))
              Text(approval.action.cwd + "\n" + argvText(approval.action.argv)).font(
                .system(.caption, design: .monospaced))
              if approval.state == "pending" {
                HStack {
                  Button("Approve exact action") { decide(approval, true) }
                  Button("Deny") { decide(approval, false) }
                }
              }
            }
          }
        }
      }
    }.padding(18).frame(width: 440, height: 500)
  }
  func decide(_ approval: ApprovalItem, _ approve: Bool) {
    model.perform {
      _ = try await Bridge.api(
        "POST", "/v1/approvals/\(approval.id)/decision",
        ["revision": approval.revision, "approved": approve])
    }
  }
}
