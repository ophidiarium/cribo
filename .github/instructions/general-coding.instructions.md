---
applyTo: "**"
---

# General coding standards for the project

- ALWAYS read the documentation using Context7.
- Take the opportunity to refactor the code to improve readability and maintainability.
- Embrace the idea of "Don't Repeat Yourself" (DRY) and ensure that the code is as clean and efficient as possible.
- Ensure that functionality is not duplicated across multiple functions
- Always read the documentation prior to implementing new functionality. Use Context 7 to get documentation about any library necessary to implement the code.

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

5. Testing and Validation

   - Suggest running unit tests or simulations on the modified segments to confirm that the changes fix the issue without impacting overall functionality.
   - Ensure that any proposed improvements, including doc comment upgrades, integrate seamlessly with the existing codebase.

6. Rationale and Explanation

   - For every change (including comment conversions), provide a concise explanation detailing how the modification resolves the identified issue while preserving the original design and context.
   - Clearly highlight only the modifications made, ensuring that no previously validated progress is altered.

7. Contextual Analysis

   - Use all available context—such as code history, inline documentation, style guidelines—to understand the intended functionality.
   - If the role or intent behind a code segment is ambiguous, ask for clarification rather than making assumptions.

8. Targeted, Incremental Changes

   - Identify and isolate only the problematic code segments (including places where IntelliSense doc comments can replace simple comments).
   - Provide minimal code snippets that address the issue without rewriting larger sections.
   - For each suggested code change, explicitly indicate the exact location in the code (e.g., by specifying the function name, class name, line number, or section heading) where the modification should be implemented.

9. Preservation of Context
   - Maintain all developer comments, annotations, and workarounds exactly as they appear, transforming them to doc comment format only when it improves IntelliSense support.
   - Do not modify or remove any non-code context unless explicitly instructed.
   - Avoid introducing new, irrelevant comments in the code.

## How to handle a question

- If you are unsure about how to implement a specific functionality, ask the user for clarification.
- Determine all relevant functions and libraries that may be necessary to implement the functionality, then read the documentation using Context7.
- Always start by looking up any documentation that may be relevant to the question using Context7.
