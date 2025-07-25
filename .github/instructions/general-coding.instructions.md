---
applyTo: "**"
---

# General coding standards for the project

## Technical Decision-Making Requirements

**MANDATORY**: As a software-engineer AI agent, you must never factor in human limitations—such as limited time or bandwidth, business priorities, implementation complexity, research requirements, or any other "human" constraint—when making technical decisions or writing recommendations. Instead, always select and articulate the technically optimal solution, judged solely on criteria like:

- **Performance** (e.g., latency, throughput)
- **Physical resource constraints** (e.g., memory footprint, disk usage, I/O)
- **Maintainability and future-proofing** (e.g., scalability, extensibility, ease of refactoring)

Under no circumstances should you justify a design or implementation by citing "lack of time," "limited resources," "tight deadlines," or similar human factors.

## Core Standards

- Take the opportunity to refactor the code to improve readability and maintainability.
- Embrace the idea of "Don't Repeat Yourself" (DRY) and ensure that the code is as clean and efficient as possible.
- Ensure that functionality is not duplicated across multiple functions
- Always read the documentation prior to implementing new functionality. Follow the documentation research hierarchy.
- **Prefer VS Code tools over terminal commands**: When available, use integrated VS Code tools (like `run_tests`, `get_errors`) before falling back to terminal commands.

## Prohibited Coding Practice: Hardcoding Test Values in Production

- **Never** insert hardcoded literals in production code solely to satisfy a test.
- All production logic must implement genuine functionality; tests should validate real behavior, not bypass it.
- If you need to simulate or stub behavior for testing, use dedicated test files or mocking frameworks—do **not** alter production code.
- Any attempt to hardcode a test value in production code is strictly forbidden and should be treated as a critical violation.
- Violations of this policy must be reported and the offending code reverted immediately.

## Agent Directive: Enforce `.clippy.toml` Disallowed Lists

- **Before generating, editing, or refactoring any Rust code**, automatically locate and parse the project's `.clippy.toml` file.
- Extract the arrays under `disallowed-types` and `disallowed-methods`. Treat each listed `path` or `method` as an absolute prohibition.
- **Never** emit or import a type identified in `disallowed-types`. For example, if `std::collections::HashSet` appears in the list, do not generate any code that uses it—use the approved alternative (e.g., `indexmap::IndexSet`) instead.
- **Never** invoke or generate code calling a method listed under `disallowed-methods`. If a method is disallowed, replace it immediately with the approved pattern or API.
- If any disallowed type or method remains in the generated code, **treat it as a critical error**: halt code generation for that snippet, annotate the violation with the specific reason from `.clippy.toml`, and refuse to proceed until the violation is removed.
- Continuously re-validate against `.clippy.toml` whenever generating new code or applying automated fixes—do not assume a one-time check is sufficient.
- Log each check and violation in clear comments or warnings within the pull request or code review context so that maintainers immediately see why a disallowed construct was rejected.

## Tool Preference Hierarchy

When multiple tools are available for the same task, follow this preference order:

1. **VS Code Integrated Tools** (Highest Priority)
   - `run_tests` for running tests
   - `get_errors` for error checking, diagnostics, and linting
   - Built-in formatting and IntelliSense features
   - **Benefits**: Better integration, formatted output, precise error locations, real-time feedback

2. **Terminal Commands** (Fallback)
   - Use when VS Code tools fail or are unavailable
   - Required for advanced tool options (e.g., `cargo clippy --fix`)
   - Necessary for CI/CD and automated workflows

**Rationale**: VS Code tools provide better integration with the development environment, more precise error reporting, and enhanced user experience compared to raw terminal output.

## Specialized Workflow Guidelines

### GitHub Actions Workflows

**MANDATORY** When working on GitHub Actions workflow files (`.github/workflows/*.yml`),
follow the specific guidelines in `github-actions-workflows.instructions.md`

## Guidelines

Use the following guidelines:

1. Doc Comment Enhancement for IntelliSense

   - Replace or augment simple comments with relevant doc comment syntax that is supported by IntelliSense as needed.
   - Preserve the original intent and wording of existing comments wherever possible.

2. Code Layout for Clarity

   - Place the most important or user-editable sections at the top if logically appropriate.
   - Insert headings or separators within the code to clearly delineate where customizations or key logic sections can be adjusted.

3. No Extraneous Code Comments

   - Do not include "one-off" or user-directed commentary in the code.
   - Confine all clarifications or additional suggestions to explanations outside of the code snippet.

4. Avoid Outdated or Deprecated Methods

   - Refrain from introducing or relying on obsolete or deprecated methods and libraries.
   - If the current code relies on potentially deprecated approaches, ask for clarification or provide viable, modern alternatives that align with best practices.

5. Immediate Code Removal Over Deprecation

   - **MANDATORY**: Since Serpen only exposes a binary CLI interface (not a library API), unused methods and functions MUST be removed immediately rather than annotated with `#[deprecated]` or similar markers.
   - **No deprecation annotations**: Do not use `#[deprecated]`, `#[allow(dead_code)]`, or similar annotations to preserve unused code.
   - **Binary-only interface**: This project does not maintain API compatibility for external consumers - all code must serve the current CLI functionality.
   - **Dead code elimination**: Aggressively remove any unused functions, methods, structs, or modules during refactoring.

6. Testing and Validation

   - **Prefer VS Code tools**: Use `run_tests` tool for running tests when available, fall back to terminal commands only if needed
   - Suggest running unit tests or simulations on the modified segments to confirm that the changes fix the issue without impacting overall functionality.
   - Ensure that any proposed improvements, including doc comment upgrades, integrate seamlessly with the existing codebase.

7. Rationale and Explanation

   - For every change (including comment conversions), provide a concise explanation detailing how the modification resolves the identified issue while preserving the original design and context.
   - Clearly highlight only the modifications made, ensuring that no previously validated progress is altered.

8. Contextual Analysis

   - Use all available context—such as code history, inline documentation, style guidelines—to understand the intended functionality.
   - If the role or intent behind a code segment is ambiguous, ask for clarification rather than making assumptions.

9. Targeted, Incremental Changes

   - Identify and isolate only the problematic code segments (including places where IntelliSense doc comments can replace simple comments).
   - Provide minimal code snippets that address the issue without rewriting larger sections.
   - For each suggested code change, explicitly indicate the exact location in the code (e.g., by specifying the function name, class name, line number, or section heading) where the modification should be implemented.

10. Preservation of Context

- Maintain all developer comments, annotations, and workarounds exactly as they appear, transforming them to doc comment format only when it improves IntelliSense support.
- Do not modify or remove any non-code context unless explicitly instructed.
- Avoid introducing new, irrelevant comments in the code.

11. Environment Variable Documentation

- **MANDATORY**: When adding support for any new environment variable, you MUST update the environment variables reference document at `docs/environment_variables.md`
- **Required Documentation Elements**:
  - Add the variable to the summary table in the "Serpen Environment Variables" section
  - Create a detailed variable reference entry with:
    - Variable type (Serpen-specific or Standard)
    - Official documentation links (for standard variables)
    - Purpose and behavior in Serpen
    - Format and usage examples
    - Platform-specific considerations if applicable
- **Integration Requirements**: Ensure the new variable integrates properly with existing environment variable handling patterns
- **Testing**: Update or add tests that verify the environment variable behavior, including edge cases

## How to handle a question

- If you are unsure about how to implement a specific functionality, ask the user for clarification.
- Follow the **Tool Preference Hierarchy**: Use VS Code tools (`run_tests`, `get_errors`) before terminal commands when both options are available.
- **MANDATORY**: Follow the **Documentation Research Hierarchy** in order - you MUST NOT skip steps without documenting why:
  1. **FIRST**: Local documentation generation and examination (`cargo doc` for Rust projects)
  2. **SECOND**: Context7 for external libraries (only if local docs insufficient)
  3. **FINAL**: GitHub MCP server tools for implementation patterns (only if steps 1&2 insufficient)
- **ENFORCEMENT**: You MUST explicitly document what you found (or didn't find) at each step before proceeding to the next
- **VIOLATION**: Jumping to GitHub MCP search without first checking local docs and Context7 violates project standards
- **Use GitHub MCP code search** ONLY as a last resort when you need implementation patterns after exhausting documentation sources. Use `f1e_search_code` to find real-world usage examples across GitHub repositories, which can provide insights into common patterns, best practices, and edge cases.
- **MANDATORY**: Document which research method provided the key insights for the implementation and why previous methods were insufficient.
