import AppKit
import SwiftUI

struct ProjectItem: Decodable, Identifiable {
  let id, name, repo_path: String
  let trusted_local: Bool
}
struct RunItem: Decodable, Identifiable {
  let id, project_id, state: String
  let revision: Int
  let plan_version: Int?
}
struct Inventory: Decodable {
  let projects: [ProjectItem]
  let runs: [RunItem]
  let project_count, run_count: Int
}
struct TaskItem: Decodable, Identifiable {
  let id, state: String
  let attempt: Int
  let spec: TaskSpec
}
struct TaskSpec: Codable {
  let id, title, description: String
  let use_case: String?
  let candidate_paths: [String]
  let acceptance: [Acceptance]
  let limits: Limits
}
struct Acceptance: Codable {
  let kind: String
  let command: [String]?
  let description: String?
}
struct Limits: Codable {
  let max_tool_calls, max_output_tokens, timeout_seconds: Int
  let cost_cap_usd: Double
}
struct PlanRecord: Decodable {
  let version: Int
  let status: String
  let plan: PlanData
}
struct PlanData: Decodable {
  let idea: String
  let requirements: [String]
  let tasks: [TaskSpec]
}
struct ProviderItem: Decodable, Identifiable { let id, kind, base_url: String }
struct ModelItem: Decodable, Identifiable {
  let id, name, provider, source: String
  let ready: Bool
}
struct Settings: Decodable {
  let providers: [ProviderItem]
  let models: [ModelItem]
  let routes: [String: String]
  let run_cap_usd, daily_cap_usd: Double
}
struct EventItem: Decodable, Identifiable {
  var id: Int { sequence }
  let sequence: Int
  let task_id, kind, to_state: String?
}
struct TestEvidence: Decodable, Identifiable {
  let id: Int
  let command: [String]
  let exit_code: Int?
  let duration_ms: Int
}
struct CommitEvidence: Decodable {
  let task_commit_sha: String
  let integration_commit_sha: String?
  let diff_paths: [String]
}
struct Evidence: Decodable {
  let commit: CommitEvidence?
  let tests: [TestEvidence]
}
struct ApprovalItem: Decodable, Identifiable {
  let id, task_id, state: String
  let revision: Int
  let action: ApprovalAction
}
struct ApprovalAction: Decodable {
  let operation, cwd: String
  let argv: [String]
}

struct Bridge {
  static var executable: URL {
    if let index = CommandLine.arguments.firstIndex(of: "--nova-path"),
      CommandLine.arguments.count > index + 1
    {
      return URL(fileURLWithPath: CommandLine.arguments[index + 1])
    }
    return Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent(
      "../Resources/nova"
    ).standardizedFileURL
  }
  static var directory: String? {
    guard let index = CommandLine.arguments.firstIndex(of: "--daemon-dir"),
      CommandLine.arguments.count > index + 1
    else { return Bundle.main.object(forInfoDictionaryKey: "NovaDataDirectory") as? String }
    return CommandLine.arguments[index + 1]
  }
  static func call(_ request: [String: Any]) async throws -> Data {
    let input = try JSONSerialization.data(withJSONObject: request)
    return try await withCheckedThrowingContinuation { continuation in
      DispatchQueue.global(qos: .userInitiated).async {
        do {
          let process = Process()
          process.executableURL = executable
          process.arguments =
            (directory.map { ["--daemon-dir", $0] } ?? []) + ["__desktop-request"]
          process.environment = ProcessInfo.processInfo.environment.filter { entry in
            !["API_KEY", "TOKEN", "SECRET"].contains(where: { entry.key.uppercased().contains($0) })
          }
          let stdin = Pipe()
          let stdout = Pipe()
          let stderr = Pipe()
          process.standardInput = stdin
          process.standardOutput = stdout
          process.standardError = stderr
          try process.run()
          // Requests are bounded; reading stdout starts before the input is written.
          let group = DispatchGroup()
          group.enter()
          var errorData = Data()
          DispatchQueue.global().async {
            errorData = stderr.fileHandleForReading.readDataToEndOfFile()
            group.leave()
          }
          try stdin.fileHandleForWriting.write(contentsOf: input)
          try stdin.fileHandleForWriting.close()
          let output = stdout.fileHandleForReading.readDataToEndOfFile()
          process.waitUntilExit()
          group.wait()
          guard process.terminationStatus == 0 else {
            let message =
              String(data: errorData.prefix(8192), encoding: .utf8)
              ?? "Nova could not complete the request."
            throw NSError(domain: "Nova", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
          }
          continuation.resume(returning: output)
        } catch { continuation.resume(throwing: error) }
      }
    }
  }
  static func api(_ method: String, _ path: String, _ body: [String: Any]? = nil) async throws
    -> Data
  {
    var value: [String: Any] = ["op": "api", "method": method, "path": path]
    if let body { value["body"] = body }
    return try await call(value)
  }
  static func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
    try JSONDecoder().decode(type, from: data)
  }
  static func pretty(_ data: Data) -> String {
    guard let value = try? JSONSerialization.jsonObject(with: data),
      let formatted = try? JSONSerialization.data(
        withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    else { return "No data" }
    return String(data: formatted, encoding: .utf8) ?? "No data"
  }
}

@MainActor final class NovaModel: ObservableObject {
  @Published var projects: [ProjectItem] = []
  @Published var runs: [RunItem] = []
  @Published var tasks: [TaskItem] = []
  @Published var settings: Settings?
  @Published var selectedRun: RunItem?
  @Published var plan: PlanRecord?
  @Published var events: [EventItem] = []
  @Published var approvals: [ApprovalItem] = []
  @Published var usage = "No provider calls recorded."
  @Published var evidence: Evidence?
  @Published var section = "Overview"
  @Published var error: String?
  @Published var notice: String?
  @Published var busy = false
  @Published var refreshing = false
  @Published var projectCount = 0
  @Published var runCount = 0
  @Published var offset = 0
  @Published var connected = false
  func refresh() async {
    guard !refreshing else { return }
    refreshing = true
    defer { refreshing = false }
    do {
      let inventory = try Bridge.decode(
        Inventory.self, await Bridge.call(["op": "snapshot", "offset": offset]))
      projects = inventory.projects
      runs = inventory.runs
      projectCount = inventory.project_count
      runCount = inventory.run_count
      connected = true
      settings = try Bridge.decode(Settings.self, await Bridge.call(["op": "settings"]))
      if let selected = selectedRun, let latest = runs.first(where: { $0.id == selected.id }) {
        selectedRun = latest
      }
      if let selected = selectedRun { try await loadRun(selected) }
    } catch {
      connected = false
      self.error = error.localizedDescription
    }
  }
  func select(_ run: RunItem) {
    selectedRun = run
    tasks = []
    plan = nil
    evidence = nil
    events = []
    approvals = []
    section = "Runs"
    Task { do { try await loadRun(run) } catch { self.error = error.localizedDescription } }
  }
  func loadRun(_ run: RunItem) async throws {
    let taskData = try await Bridge.api("GET", "/v1/runs/\(run.id)/tasks")
    let newTasks = try Bridge.decode([TaskItem].self, taskData)
    let newEvents = try Bridge.decode(
      [EventItem].self, await Bridge.api("GET", "/v1/events?run_id=\(run.id)&after=0&limit=1000"))
    let newApprovals = try Bridge.decode(
      [ApprovalItem].self, await Bridge.api("GET", "/v1/approvals?run_id=\(run.id)"))
    let newUsage = try await Bridge.api("GET", "/v1/usage?run_id=\(run.id)")
    var newPlan: PlanRecord?
    if let version = run.plan_version {
      newPlan = try Bridge.decode(
        PlanRecord.self, await Bridge.api("GET", "/v1/plans/\(run.id)/\(version)"))
    }
    guard selectedRun?.id == run.id else { return }
    tasks = newTasks
    events = newEvents
    approvals = newApprovals
    usage = Bridge.pretty(newUsage)
    plan = newPlan
  }
  func perform(_ action: @escaping () async throws -> Void) {
    guard !busy else { return }
    busy = true
    error = nil
    notice = nil
    Task {
      defer { busy = false }
      do {
        try await action()
        await refresh()
      } catch { self.error = error.localizedDescription }
    }
  }
  func control(_ action: String) {
    guard let run = selectedRun else { return }
    perform {
      _ = try await Bridge.api("POST", "/v1/runs/\(run.id)/\(action)")
      self.notice = "Run \(action) request recorded."
    }
  }
  func inspect(_ task: TaskItem) {
    guard let run = selectedRun else { return }
    perform {
      self.evidence = try Bridge.decode(
        Evidence.self, await Bridge.api("GET", "/v1/tasks/\(run.id)/\(task.id)/evidence"))
    }
  }
}

let accent = Color(red: 0.90, green: 0.32, blue: 0.17)
let useCases = ["coding", "planning", "review", "debugging", "architecture", "fast"]
func stateColor(_ state: String) -> Color {
  if state == "completed" { return .green }
  if ["blocked", "failed", "cancelled"].contains(state) { return .orange }
  if ["running", "leased", "verifying", "integrating"].contains(state) { return accent }
  return .secondary
}
struct StatePill: View {
  let state: String
  var body: some View {
    Text(state.replacingOccurrences(of: "_", with: " ").capitalized).font(.caption.weight(.medium))
      .padding(.horizontal, 9).padding(.vertical, 4).background(stateColor(state).opacity(0.12))
      .foregroundStyle(stateColor(state)).clipShape(Capsule())
  }
}
struct Card<Content: View>: View {
  @ViewBuilder let content: Content
  var body: some View {
    VStack(alignment: .leading, spacing: 14) { content }.padding(20).frame(
      maxWidth: .infinity, alignment: .leading
    ).background(Color(nsColor: .controlBackgroundColor)).clipShape(
      RoundedRectangle(cornerRadius: 14)
    ).overlay(RoundedRectangle(cornerRadius: 14).stroke(Color.primary.opacity(0.07)))
  }
}

@main struct NovaDesktop: App {
  @StateObject private var model = NovaModel()
  var body: some Scene {
    WindowGroup("Nova") {
      Dashboard().environmentObject(model).frame(minWidth: 1000, minHeight: 700).tint(accent).task {
        do { _ = try await Bridge.call(["op": "start"]) } catch {
          model.error = error.localizedDescription
        }
        await model.refresh()
      }.task {
        while !Task.isCancelled {
          try? await Task.sleep(nanoseconds: 4_000_000_000)
          if !model.busy { await model.refresh() }
        }
      }
    }.defaultSize(width: 1220, height: 820).commands { CommandGroup(replacing: .newItem) {} }
  }
}
struct Dashboard: View {
  @EnvironmentObject var model: NovaModel
  @State private var projectForm = false
  @State private var planForm = false
  @State private var providerForm = false
  @State private var executionReview = false
  @State private var approveReview = false
  @State private var cancelReview = false
  var body: some View {
    HStack(spacing: 0) {
      VStack(alignment: .leading, spacing: 24) {
        HStack(spacing: 10) {
          Image(systemName: "sparkle").font(.title).foregroundStyle(accent)
          Text("nova").font(.system(size: 29, weight: .bold, design: .rounded))
        }.padding(.top, 18)
        Text("YOUR AGENT WORKSPACE").font(.system(size: 9, weight: .semibold)).tracking(1.5)
          .foregroundStyle(.secondary)
        ForEach(
          [
            ("Overview", "square.grid.2x2"), ("Runs", "rectangle.stack"), ("Providers", "network"),
            ("Model routes", "arrow.triangle.branch"),
          ], id: \.0
        ) { item in
          Button {
            model.section = item.0
          } label: {
            Label(item.0, systemImage: item.1).font(.system(size: 14, weight: .medium)).padding(11)
              .frame(maxWidth: .infinity, alignment: .leading).background(
                model.section == item.0 ? accent.opacity(0.1) : Color.clear
              ).clipShape(RoundedRectangle(cornerRadius: 9))
          }.buttonStyle(.plain).foregroundStyle(model.section == item.0 ? accent : .primary)
        }
        Spacer()
        VStack(alignment: .leading, spacing: 8) {
          Label(
            model.connected ? "Local service connected" : "Service unavailable",
            systemImage: model.connected ? "circle.fill" : "circle"
          ).font(.caption).foregroundStyle(model.connected ? .green : .secondary)
          Text(
            "Nova runs locally. Live models receive task context. Closing this window leaves Nova running."
          ).font(
            .caption
          ).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
      }.padding(22).frame(width: 220).background(Color(nsColor: .windowBackgroundColor))
      Divider()
      VStack(spacing: 0) {
        HStack {
          VStack(alignment: .leading, spacing: 4) {
            Text(model.section).font(.system(size: 27, weight: .semibold))
            Text(subtitle).font(.callout).foregroundStyle(.secondary)
          }
          Spacer()
          if model.busy || model.refreshing { ProgressView().controlSize(.small) }
          Button {
            Task { await model.refresh() }
          } label: {
            Image(systemName: "arrow.clockwise")
          }.help("Refresh persisted state")
          primaryButton
        }.padding(26)
        Divider()
        if let error = model.error {
          HStack {
            Image(systemName: "exclamationmark.triangle")
            Text(error).font(.callout).textSelection(.enabled)
            Spacer()
            Button("Dismiss") { model.error = nil }
          }.padding(14).background(Color.orange.opacity(0.1))
        }
        if let notice = model.notice {
          HStack {
            Image(systemName: "checkmark.circle")
            Text(notice).font(.callout)
            Spacer()
            Button {
              model.notice = nil
            } label: {
              Image(systemName: "xmark")
            }
          }.padding(12).background(Color.green.opacity(0.08))
        }
        ScrollView {
          VStack(alignment: .leading, spacing: 20) {
            switch model.section {
            case "Providers": providers
            case "Model routes": routes
            case "Runs": runs
            default: overview
            }
          }.padding(26).frame(maxWidth: .infinity, alignment: .leading)
        }
      }.background(Color(nsColor: .underPageBackgroundColor))
    }.disabled(model.busy)
      .sheet(isPresented: $projectForm) { ProjectForm().environmentObject(model) }
      .sheet(isPresented: $planForm) { PlanForm().environmentObject(model) }
      .sheet(isPresented: $providerForm) { ProviderForm().environmentObject(model) }
      .sheet(isPresented: $executionReview) { ExecuteReview().environmentObject(model) }
      .confirmationDialog(
        "Approve this plan version?", isPresented: $approveReview, titleVisibility: .visible
      ) {
        Button("Approve reviewed plan") {
          guard let run = model.selectedRun, let plan = model.plan else { return }
          model.perform {
            _ = try await Bridge.api("POST", "/v1/plans/\(run.id)/\(plan.version)/approve")
            model.notice = "Plan approved. Execution still requires your separate action."
          }
        }
      } message: {
        Text(
          "This authorizes the task scope and budgets shown below. No workers start until you choose Execute."
        )
      }
      .confirmationDialog("Cancel this run?", isPresented: $cancelReview, titleVisibility: .visible)
    {
      Button("Cancel run", role: .destructive) { model.control("cancel") }
    } message: {
      Text(
        "Queued tasks will be cancelled and active workers signalled. Retained branches and evidence remain available."
      )
    }
  }
  var subtitle: String {
    switch model.section {
    case "Providers": return "Connect accounts once. Keep credentials in your OS vault."
    case "Model routes": return "Choose the model expertise each kind of task uses."
    case "Runs": return "Follow tasks, approvals, and verified results."
    default: return "A clear view of your projects and agent work."
    }
  }
  @ViewBuilder var primaryButton: some View {
    if model.section == "Providers" {
      Button("Connect provider", systemImage: "plus") { providerForm = true }.buttonStyle(
        .borderedProminent)
    } else {
      Button("Create task plan", systemImage: "plus") { planForm = true }.buttonStyle(
        .borderedProminent
      ).disabled(model.projects.isEmpty)
    }
  }
  var overview: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack(spacing: 16) {
        metric("Projects", value: String(model.projectCount), icon: "folder")
        metric("Runs", value: String(model.runCount), icon: "rectangle.stack")
        metric(
          "Active on this page",
          value: String(model.runs.filter { ["running", "paused"].contains($0.state) }.count),
          icon: "waveform.path")
        metric("Providers", value: String(model.settings?.providers.count ?? 0), icon: "network")
      }
      if model.projects.isEmpty {
        Card {
          Text("Start your first project").font(.title2.weight(.semibold))
          Text(
            "Choose a local Git repository, describe a task, and review its scope before starting agents."
          ).foregroundStyle(.secondary)
          Button("Add project", systemImage: "folder.badge.plus") { projectForm = true }
            .buttonStyle(.borderedProminent)
        }
      }
      HStack {
        Text("Projects").font(.title3.weight(.semibold))
        Spacer()
        Button("Add project") { projectForm = true }
      }
      ForEach(model.projects) { project in
        Card {
          HStack {
            Image(systemName: "folder").foregroundStyle(accent)
            Text(project.name).font(.headline)
            Spacer()
            StatePill(state: project.trusted_local ? "trusted local" : "execution disabled")
          }
          Text(project.repo_path).font(.system(.caption, design: .monospaced)).foregroundStyle(
            .secondary
          ).textSelection(.enabled)
        }
      }
      Text("Recent runs").font(.title3.weight(.semibold))
      runList
      if model.projects.count < model.projectCount || model.runs.count < model.runCount {
        pagination
      }
    }
  }
  func metric(_ title: String, value: String, icon: String) -> some View {
    Card {
      HStack {
        Text(title).font(.caption).foregroundStyle(.secondary)
        Spacer()
        Image(systemName: icon).foregroundStyle(accent)
      }
      Text(value).font(.system(size: 30, weight: .semibold, design: .rounded))
    }
  }
  var runList: some View {
    VStack(spacing: 10) {
      if model.runs.isEmpty {
        Card {
          Text("No runs yet").font(.headline)
          Text("Create a task plan to start a reviewable run.").foregroundStyle(.secondary)
        }
      }
      ForEach(model.runs) { run in
        Button {
          model.select(run)
        } label: {
          HStack {
            VStack(alignment: .leading, spacing: 5) {
              Text(model.projects.first(where: { $0.id == run.project_id })?.name ?? run.project_id)
                .font(.headline)
              Text(String(run.id.prefix(12))).font(.system(.caption, design: .monospaced))
                .foregroundStyle(.secondary)
            }
            Spacer()
            StatePill(state: run.state)
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.secondary)
          }.padding(16).background(Color(nsColor: .controlBackgroundColor)).clipShape(
            RoundedRectangle(cornerRadius: 10))
        }.buttonStyle(.plain)
      }
    }
  }
  var pagination: some View {
    HStack {
      Button("Previous page") {
        model.offset = max(0, model.offset - 100)
        Task { await model.refresh() }
      }.disabled(model.offset == 0)
      Text("Showing page \(model.offset/100+1)").font(.caption).foregroundStyle(.secondary)
      Button("Next page") {
        model.offset += 100
        Task { await model.refresh() }
      }.disabled(model.offset + 100 >= max(model.projectCount, model.runCount))
    }
  }
  var runs: some View {
    VStack(alignment: .leading, spacing: 20) {
      if let run = model.selectedRun {
        Button("All runs", systemImage: "arrow.left") {
          model.selectedRun = nil
          model.evidence = nil
        }
        Card {
          HStack {
            VStack(alignment: .leading, spacing: 6) {
              Text(model.projects.first(where: { $0.id == run.project_id })?.name ?? "Run").font(
                .title2.weight(.semibold))
              Text(run.id).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            }
            Spacer()
            StatePill(state: run.state)
          }
          if let plan = model.plan {
            Text(plan.plan.idea).font(.headline)
            Text("Plan version \(plan.version) · \(plan.status)").font(.caption).foregroundStyle(
              .secondary)
          }
          HStack {
            if run.state == "awaiting_approval" {
              Button("Approve plan") { approveReview = true }.buttonStyle(.borderedProminent)
            }
            if run.state == "running" {
              Button("Execute…") { executionReview = true }.buttonStyle(.borderedProminent)
              Button("Pause") { model.control("pause") }
            }
            if run.state == "paused" { Button("Resume eligibility") { model.control("resume") } }
            if !["completed", "cancelled", "failed"].contains(run.state) {
              Button("Cancel run", role: .destructive) { cancelReview = true }
            }
          }
          Text(
            "Pause prevents new assignments; current work can finish. Resume restores eligibility. Execute starts the worker loop."
          ).font(.caption).foregroundStyle(.secondary)
        }
        let shown =
          model.tasks.isEmpty
          ? (model.plan?.plan.tasks.map {
            TaskItem(id: $0.id, state: "proposed", attempt: 0, spec: $0)
          } ?? []) : model.tasks
        HStack {
          Text("Tasks").font(.title3.weight(.semibold))
          Spacer()
          Text(
            "\(model.tasks.filter{$0.state=="completed"}.count) / \(shown.count) verified and integrated"
          ).font(.caption).foregroundStyle(.secondary)
        }
        ForEach(shown) { task in
          Card {
            HStack {
              Text(task.spec.title).font(.headline)
              Spacer()
              StatePill(state: task.state)
            }
            Text(task.spec.description).foregroundStyle(.secondary)
            HStack {
              Label(task.spec.use_case ?? "coding", systemImage: "sparkle")
              Text("Attempt \(task.attempt)")
              Spacer()
              Text(String(format: "Task cap $%.2f", task.spec.limits.cost_cap_usd))
            }.font(.caption).foregroundStyle(.secondary)
            Text(task.spec.candidate_paths.joined(separator: "\n")).font(
              .system(.caption, design: .monospaced)
            ).textSelection(.enabled)
            DisclosureGroup("Acceptance commands") {
              ForEach(Array(task.spec.acceptance.enumerated()), id: \.offset) { item in
                Text(
                  item.element.command.map { argvText($0) } ?? item.element.description
                    ?? "Assertion"
                ).font(.system(.caption, design: .monospaced)).frame(
                  maxWidth: .infinity, alignment: .leading
                ).textSelection(.enabled).padding(.top, 5)
              }
            }
            Button("Inspect verification evidence") { model.inspect(task) }
          }
        }
        if let evidence = model.evidence { evidenceCard(evidence) }
        ForEach(model.approvals) { approval in
          Card {
            HStack {
              Text("Command approval").font(.headline)
              Spacer()
              StatePill(state: approval.state)
            }
            Text("Task: \(approval.task_id)").font(.caption)
            Text(
              approval.action.operation + "\n" + approval.action.cwd + "\n"
                + argvText(approval.action.argv)
            ).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            if approval.state == "pending" {
              HStack {
                Button("Approve exact action") { decide(approval, true) }
                Button("Deny", role: .destructive) { decide(approval, false) }
              }
              Text(
                "Applies only to this exact scoped action and attempt. A blocked task may still need a revised run."
              ).font(.caption).foregroundStyle(.secondary)
            }
          }
        }
        Card {
          Text("Usage & cost").font(.headline)
          Text("Unknown measurements remain unknown. Listed costs are estimates.").font(.caption)
            .foregroundStyle(.secondary)
          Text(model.usage).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
        }
        Card {
          Text("Recorded activity").font(.headline)
          Text("First 1,000 durable events in this run.").font(.caption).foregroundStyle(.secondary)
          ForEach(model.events.suffix(100).reversed()) { event in
            HStack {
              Text("#\(event.sequence)").foregroundStyle(.tertiary).frame(
                width: 45, alignment: .leading)
              Text(event.task_id ?? "Run")
              Spacer()
              Text(event.to_state ?? event.kind ?? "Event").foregroundStyle(.secondary)
            }.font(.caption)
          }
        }
      } else {
        runList
        pagination
      }
    }
  }
  func decide(_ approval: ApprovalItem, _ approve: Bool) {
    model.perform {
      _ = try await Bridge.api(
        "POST", "/v1/approvals/\(approval.id)/decision",
        ["revision": approval.revision, "approved": approve])
      model.notice = "Approval decision recorded."
    }
  }
  func evidenceCard(_ evidence: Evidence) -> some View {
    Card {
      Text("Verification evidence").font(.headline)
      if let commit = evidence.commit {
        Text(
          "Task commit: \(commit.task_commit_sha)\nIntegrated commit: \(commit.integration_commit_sha ?? "Not integrated")"
        ).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
      } else {
        Text("No task commit recorded.").foregroundStyle(.secondary)
      }
      if evidence.tests.isEmpty {
        Text("No verifier results recorded.").foregroundStyle(.secondary)
      }
      ForEach(evidence.tests) { test in
        HStack {
          VStack(alignment: .leading) {
            Text(argvText(test.command)).font(.system(.caption, design: .monospaced)).textSelection(
              .enabled)
            Text("\(test.duration_ms) ms").font(.caption).foregroundStyle(.secondary)
          }
          Spacer()
          StatePill(
            state: test.exit_code == 0 ? "passed" : test.exit_code == nil ? "unknown" : "failed")
        }
      }
    }
  }
  var providers: some View {
    VStack(alignment: .leading, spacing: 20) {
      if model.settings?.providers.isEmpty ?? true {
        Card {
          Text("Bring your models together").font(.title2.weight(.semibold))
          Text(
            "Connect OpenAI, Claude, or an OpenAI-compatible endpoint. Nova discovers models without generating tokens."
          ).foregroundStyle(.secondary)
          Button("Connect your first provider") { providerForm = true }.buttonStyle(
            .borderedProminent)
        }
      }
      ForEach(model.settings?.providers ?? []) { provider in
        ProviderCard(provider: provider).environmentObject(model)
      }
      Card {
        Text("Model availability").font(.headline)
        Text(
          "Models need known prices, token limits, and structured output support. Unknown metadata stays unavailable. Configure additional metadata with nova models configure."
        ).font(.callout).foregroundStyle(.secondary)
        ForEach(model.settings?.models ?? []) { item in
          HStack {
            VStack(alignment: .leading, spacing: 4) {
              Text(item.id).font(.system(.callout, design: .monospaced))
              Text(item.source).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            StatePill(state: item.ready ? "ready" : "needs metadata")
          }
        }
      }
    }
  }
  var routes: some View {
    VStack(alignment: .leading, spacing: 20) {
      Card {
        Text("Choose by use case").font(.title2.weight(.semibold))
        Text(
          "Pin a use case to a model. Unpinned tasks use the saved complexity and risk defaults; Nova does not benchmark expertise automatically."
        ).foregroundStyle(.secondary)
        Text(
          String(
            format: "Default run cap $%.2f · Daily cap $%.2f", model.settings?.run_cap_usd ?? 5,
            model.settings?.daily_cap_usd ?? 10)
        ).font(.caption)
      }
      ForEach(useCases, id: \.self) { role in RouteRow(role: role).environmentObject(model) }
    }
  }
}
func argvText(_ argv: [String]) -> String {
  argv.map { "\"" + $0.replacingOccurrences(of: "\"", with: "\\\"") + "\"" }.joined(separator: " ")
}
struct ProviderCard: View {
  @EnvironmentObject var model: NovaModel
  let provider: ProviderItem
  @State var removing = false
  var body: some View {
    Card {
      HStack {
        Image(systemName: "network").foregroundStyle(accent)
        Text(provider.id).font(.headline)
        Spacer()
        Text(provider.kind).font(.caption).foregroundStyle(.secondary)
        Button("Remove", role: .destructive) { removing = true }
      }
      Text(provider.base_url).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
    }.confirmationDialog(
      "Remove \(provider.id)?", isPresented: $removing, titleVisibility: .visible
    ) {
      Button("Remove connection and saved key", role: .destructive) {
        model.perform {
          _ = try await Bridge.call(["op": "remove", "name": provider.id])
          model.notice = "Provider removed."
        }
      }
    } message: {
      Text(
        "Its models and use-case routes will be removed. Running executions retain their current configuration."
      )
    }
  }
}
struct RouteRow: View {
  @EnvironmentObject var model: NovaModel
  let role: String
  @State var selected = ""
  var body: some View {
    Card {
      HStack {
        Text(role.capitalized).font(.headline).frame(width: 115, alignment: .leading)
        Picker("Model", selection: $selected) {
          Text("Use complexity / risk defaults").tag("")
          ForEach((model.settings?.models ?? []).filter { $0.ready }) { Text($0.id).tag($0.id) }
        }.labelsHidden()
        Button("Save route") {
          let id = selected
          model.perform {
            _ = try await Bridge.call(["op": "route", "use_case": role, "model": id])
            model.notice = "\(role.capitalized) route saved."
          }
        }
      }
      Text(
        model.settings?.routes[role].map { "Pinned to \($0)" } ?? "Using complexity / risk defaults"
      ).font(.caption).foregroundStyle(.secondary)
    }.onAppear { selected = model.settings?.routes[role] ?? "" }
  }
}

struct SheetHeader: View {
  let title, subtitle: String
  var body: some View {
    VStack(alignment: .leading, spacing: 7) {
      Text(title).font(.title2.weight(.semibold))
      Text(subtitle).font(.callout).foregroundStyle(.secondary)
    }.frame(maxWidth: .infinity, alignment: .leading)
  }
}
struct ProjectForm: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  @State var name = ""
  @State var path = ""
  @State var trusted = false
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      SheetHeader(
        title: "Add a project", subtitle: "Choose an existing Git repository on this Mac.")
      TextField("Project name", text: $name).textFieldStyle(.roundedBorder)
      HStack {
        TextField("Repository folder", text: $path).textFieldStyle(.roundedBorder)
        Button("Choose…") {
          let panel = NSOpenPanel()
          panel.canChooseDirectories = true
          panel.canChooseFiles = false
          if panel.runModal() == .OK, let url = panel.url {
            path = url.path
            if name.isEmpty { name = url.lastPathComponent }
          }
        }
      }
      Toggle("I trust this repository and authorize its local code to execute", isOn: $trusted)
      Text(
        "Repository tests and commands run with your user permissions. Git worktrees isolate changes, but provide no OS sandbox. Without trust, this project can be registered but cannot execute."
      ).font(.caption).foregroundStyle(.secondary)
      HStack {
        Button("Cancel") { dismiss() }
        Spacer()
        Button("Add project") {
          let body: [String: Any] = ["name": name, "repo_path": path, "trusted_local": trusted]
          model.perform {
            _ = try await Bridge.api("POST", "/v1/projects", body)
            model.notice = "Project added."
          }
          dismiss()
        }.buttonStyle(.borderedProminent).disabled(
          name.trimmingCharacters(in: .whitespaces).isEmpty || path.isEmpty)
      }
    }.padding(28).frame(width: 590)
  }
}
struct ProviderForm: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  @State var preset = "openai"
  @State var name = "openai"
  @State var endpoint = "https://api.openai.com/v1"
  @State var key = ""
  @State var python = ""
  var body: some View {
    VStack(alignment: .leading, spacing: 19) {
      SheetHeader(
        title: "Connect a provider",
        subtitle:
          "Your key is stored in macOS Keychain. Setup lists models without generating tokens.")
      Picker("Provider", selection: $preset) {
        Text("OpenAI").tag("openai")
        Text("Claude / Anthropic").tag("anthropic")
        Text("OpenRouter").tag("openrouter")
        Text("Compatible endpoint").tag("compatible")
      }.onChange(of: preset) { _, value in
        name = value
        endpoint =
          value == "openai"
          ? "https://api.openai.com/v1"
          : value == "anthropic"
            ? "https://api.anthropic.com/v1"
            : value == "openrouter" ? "https://openrouter.ai/api/v1" : ""
      }
      TextField("Connection name", text: $name).textFieldStyle(.roundedBorder)
      TextField("API base URL", text: $endpoint).textFieldStyle(.roundedBorder).disabled(
        ["openai", "anthropic"].contains(preset))
      SecureField("API key", text: $key).textFieldStyle(.roundedBorder)
      DisclosureGroup("Advanced: intelligence Python") {
        TextField("Optional absolute venv Python path", text: $python).textFieldStyle(
          .roundedBorder)
        Text(
          "Nova finds the installed runtime automatically. Supply a path if you use a separate installation."
        ).font(.caption).foregroundStyle(.secondary)
      }
      Text(
        "Use an API key from this provider. A chat subscription may have separate API billing. The OS may ask you to unlock or allow Keychain access."
      ).font(.caption).foregroundStyle(.secondary)
      HStack {
        Button("Cancel") {
          key = ""
          dismiss()
        }
        Spacer()
        Button("Connect & discover models") {
          var request: [String: Any] = [
            "op": "connect", "name": name,
            "kind": ["openai", "anthropic"].contains(preset) ? preset : "compatible",
            "base_url": endpoint, "key": key,
          ]
          if !python.isEmpty { request["python"] = python }
          key = ""
          model.perform {
            let result = try await Bridge.call(request)
            let response = try JSONSerialization.jsonObject(with: result) as? [String: Any]
            model.notice = response?["message"] as? String
          }
          dismiss()
        }.buttonStyle(.borderedProminent).disabled(key.isEmpty || name.isEmpty || endpoint.isEmpty)
      }
    }.padding(28).frame(width: 610).onDisappear { key = "" }
  }
}
struct TaskDraft: Identifiable {
  var id = "task-" + UUID().uuidString.lowercased().prefix(8), title = "", description = "",
    paths = "", arguments = "python3\n-m\npytest\n-q", useCase = "coding", complexity = "medium",
    risk = "low", cost = 0.5, sequential = true
  var argv: [String] { arguments.components(separatedBy: "\n").filter { !$0.isEmpty } }
  var scope: [String] {
    paths.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.filter {
      !$0.isEmpty
    }
  }
  var valid: Bool {
    title.count >= 3 && description.count >= 10 && !scope.isEmpty && !argv.isEmpty && cost > 0
      && cost.isFinite
  }
  func proposal(previous: String?) -> [String: Any] {
    [
      "id": id, "title": title, "description": description, "use_case": useCase,
      "complexity": complexity, "risk": risk, "agent_priority": 50, "manual_priority": NSNull(),
      "dependencies": sequential ? previous.map { [$0] } ?? [] : [], "candidate_paths": scope,
      "acceptance": [["kind": "test", "command": argv]],
      "limits": [
        "max_tool_calls": 100, "max_output_tokens": 16000, "timeout_seconds": 900,
        "cost_cap_usd": cost,
      ],
    ]
  }
}
struct PlanForm: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  @State var project = ""
  @State var idea = ""
  @State var drafts = [TaskDraft()]
  @State var selected = 0
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      SheetHeader(
        title: "Create a task plan",
        subtitle:
          "Describe the scope and exact acceptance for each task. Nova saves a draft for your review."
      )
      Picker("Project", selection: $project) {
        Text("Choose a project").tag("")
        ForEach(model.projects) { Text($0.name).tag($0.id) }
      }
      TextField("Overall goal (at least 5 characters)", text: $idea).textFieldStyle(.roundedBorder)
      HStack(alignment: .top, spacing: 20) {
        VStack(alignment: .leading, spacing: 10) {
          Text("TASKS").font(.caption).foregroundStyle(.secondary)
          ForEach(Array(drafts.enumerated()), id: \.element.id) { index, draft in
            Button {
              selected = index
            } label: {
              HStack {
                Text("\(index+1). \(draft.title.isEmpty ? "New task" : draft.title)").lineLimit(2)
                Spacer()
                if selected == index { Image(systemName: "chevron.right") }
              }.padding(9).frame(maxWidth: .infinity, alignment: .leading).background(
                selected == index ? accent.opacity(0.1) : Color.clear
              ).clipShape(RoundedRectangle(cornerRadius: 7))
            }.buttonStyle(.plain)
          }
          Button("Add task", systemImage: "plus") {
            drafts.append(TaskDraft())
            selected = drafts.count - 1
          }
          Spacer()
        }.frame(width: 190)
        Divider()
        ScrollView {
          VStack(alignment: .leading, spacing: 14) {
            TextField("Task title", text: $drafts[selected].title).textFieldStyle(.roundedBorder)
            Text("What should this task do?").font(.caption.weight(.semibold))
            TextEditor(text: $drafts[selected].description).font(.body).frame(height: 75).overlay(
              RoundedRectangle(cornerRadius: 5).stroke(.quaternary))
            HStack {
              Picker("Use case", selection: $drafts[selected].useCase) {
                ForEach(useCases, id: \.self) { Text($0.capitalized).tag($0) }
              }
              Picker("Complexity", selection: $drafts[selected].complexity) {
                ForEach(["low", "medium", "high"], id: \.self) { Text($0.capitalized).tag($0) }
              }
              Picker("Risk", selection: $drafts[selected].risk) {
                ForEach(["low", "medium", "high"], id: \.self) { Text($0.capitalized).tag($0) }
              }
            }
            Text("Allowed file paths / patterns · one per line").font(.caption.weight(.semibold))
            TextEditor(text: $drafts[selected].paths).font(.system(.callout, design: .monospaced))
              .frame(height: 65).overlay(RoundedRectangle(cornerRadius: 5).stroke(.quaternary))
            Text("Acceptance command · program first, then ONE argument per line").font(
              .caption.weight(.semibold))
            TextEditor(text: $drafts[selected].arguments).font(
              .system(.callout, design: .monospaced)
            ).frame(height: 85).overlay(RoundedRectangle(cornerRadius: 5).stroke(.quaternary))
            Text(
              "Arguments are passed literally. Do not paste a shell command or add shell quoting. Example: python3 ↵ -m ↵ pytest ↵ -q"
            ).font(.caption).foregroundStyle(.secondary)
            HStack {
              TextField("Task spend cap (USD)", value: $drafts[selected].cost, format: .number)
                .textFieldStyle(.roundedBorder)
              if selected > 0 { Toggle("After previous task", isOn: $drafts[selected].sequential) }
            }
            if drafts.count > 1 {
              Button("Remove this task", role: .destructive) {
                drafts.remove(at: selected)
                selected = min(selected, drafts.count - 1)
              }
            }
          }.padding(.trailing, 5)
        }
      }.frame(height: 440)
      Text(
        "This is your authored plan. It does not call a model to decompose the goal. Submission, approval, and execution are separate steps. Default task limits: 100 tools, 16,000 output tokens, 15 minutes."
      ).font(.caption).foregroundStyle(.secondary)
      HStack {
        Button("Cancel") { dismiss() }
        Spacer()
        Text(String(format: "Total task caps: $%.2f", drafts.reduce(0) { $0 + $1.cost })).font(
          .caption)
        Button("Save plan for review") {
          let tasks = drafts.enumerated().map { index, draft in
            draft.proposal(previous: index > 0 ? drafts[index - 1].id : nil)
          }
          let plan: [String: Any] = [
            "schema_version": "1.0", "project_id": project, "idea": idea, "requirements": [idea],
            "assumptions": ["Task scope and acceptance were authored by the operator."],
            "architecture": [
              "recommended": "Scoped changes within the existing repository architecture.",
              "alternatives": [],
              "risks": ["Local repository commands execute with user permissions."],
            ], "tasks": tasks,
          ]
          model.perform {
            let data = try await Bridge.call(["op": "create_plan", "plan": plan])
            let value = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            let run = try JSONSerialization.data(withJSONObject: value["run"]!)
            var item = try Bridge.decode(RunItem.self, run)
            item = RunItem(
              id: item.id, project_id: item.project_id, state: "awaiting_approval",
              revision: item.revision + 1, plan_version: 1)
            model.select(item)
            model.notice = "Plan saved. Review each task before approving."
          }
          dismiss()
        }.buttonStyle(.borderedProminent).disabled(
          project.isEmpty || idea.count < 5 || !drafts.allSatisfy { $0.valid })
      }
    }.padding(28).frame(width: 900).onAppear { project = model.projects.first?.id ?? "" }
  }
}
struct ExecuteReview: View {
  @EnvironmentObject var model: NovaModel
  @Environment(\.dismiss) var dismiss
  @State var live = true
  @State var acknowledged = false
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      SheetHeader(
        title: "Review execution",
        subtitle:
          "Nova will run the approved tasks, verify changes, and integrate successful task commits locally."
      )
      Toggle("Use configured live models (API billing applies)", isOn: $live)
      if !live {
        Text(
          "Offline mock mode writes synthetic fixture text to each exact task path. It cannot implement your goal. Use it only in a disposable demonstration repository."
        ).foregroundStyle(.orange).font(.callout)
      }
      if let plan = model.plan {
        ScrollView {
          VStack(alignment: .leading, spacing: 15) {
            ForEach(plan.plan.tasks, id: \.id) { task in
              VStack(alignment: .leading, spacing: 6) {
                Text(task.title).font(.headline)
                Text(
                  "Model route: \(model.settings?.routes[task.use_case ?? "coding"] ?? "complexity / risk defaults")"
                ).font(.caption).foregroundStyle(.secondary)
                Text(String(format: "Task spend cap $%.2f", task.limits.cost_cap_usd)).font(
                  .caption)
                ForEach(Array(task.acceptance.enumerated()), id: \.offset) { criterion in
                  Text(
                    criterion.element.command.map { argvText($0) } ?? criterion.element.description
                      ?? "Assertion"
                  ).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                }
              }
            }
          }.frame(maxWidth: .infinity, alignment: .leading)
        }.frame(maxHeight: 280)
      }
      Text(
        String(
          format:
            "Saved run cap $%.2f · Daily cap $%.2f. Estimated pricing can differ from provider billing.",
          model.settings?.run_cap_usd ?? 5, model.settings?.daily_cap_usd ?? 10)
      ).font(.caption).foregroundStyle(.secondary)
      Toggle("I authorize these exact test commands and this execution mode", isOn: $acknowledged)
      HStack {
        Button("Cancel") { dismiss() }
        Spacer()
        Button(live ? "Start live execution" : "Start mock demonstration") {
          guard let run = model.selectedRun, let plan = model.plan else { return }
          let request: [String: Any] = [
            "op": "execute", "run_id": run.id, "version": plan.version, "live": live,
          ]
          model.perform {
            _ = try await Bridge.call(request)
            model.notice = "Execution accepted. Watch task states for verified completion."
          }
          dismiss()
        }.buttonStyle(.borderedProminent).disabled(
          !acknowledged || (live && !(model.settings?.models.contains { $0.ready } ?? false)))
      }
    }.padding(28).frame(width: 720)
  }
}
