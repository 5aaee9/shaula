use shaula_core::registry::MutationError;

pub(super) fn check(
    current: Option<(&str, i64)>,
    create: bool,
    expected: Option<&(String, i64)>,
) -> Result<(), MutationError> {
    match (current, create, expected) {
        (_, false, None) => Err(MutationError::PreconditionRequired),
        (None, true, None) => Ok(()),
        (Some((incarnation, revision)), false, Some((want_incarnation, want_revision)))
            if incarnation == want_incarnation && revision == *want_revision =>
        {
            Ok(())
        }
        (Some((incarnation, revision)), _, _) => Err(MutationError::PreconditionFailed {
            current: (incarnation.to_string(), revision),
        }),
        _ => Err(MutationError::PreconditionRequired),
    }
}
