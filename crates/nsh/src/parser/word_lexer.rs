use super::{
    BackquoteContext, CTLESC, Error, InputUnit, MultibyteMode, Rt1, Shell, getmbc_at, pgetc,
    pungetc,
};

pub(super) fn read_backslash(sh: &mut Shell, st: &mut Rt1<'_>) -> Result<(), Error> {
    st.input = pgetc(sh)?;
    if st.input == InputUnit::EndOfInput {
        st.out.push(CTLESC as u8);
        st.out.push(b'\\');
        pungetc(sh);
        return Ok(());
    }

    if (st.syn().double_quoted || st.syn().backquote != BackquoteContext::None)
        && !st.input.is(b'\\')
        && !st.input.is(b'`')
        && !st.input.is(b'$')
        && (!st.input.is(b'"') || (!st.eofmark.is_none() && st.syn().variable_depth == 0))
        && (!st.input.is(b'}') || st.syn().variable_depth == 0)
    {
        st.out.push(CTLESC as u8);
        st.out.push(b'\\');
    }
    st.quoted = true;

    if getmbc_at(sh, &mut st.out, st.input, MultibyteMode::Escaped)? == 0 {
        st.out.push(CTLESC as u8);
        st.out.push(st.input.expect_byte());
    }
    Ok(())
}
