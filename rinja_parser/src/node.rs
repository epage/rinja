use std::str;

use nom::branch::alt;
use nom::bytes::complete::{tag, take_till};
use nom::character::complete::char;
use nom::combinator::{complete, consumed, cut, eof, map, not, opt, peek, recognize, value};
use nom::error::ErrorKind;
use nom::error_position;
use nom::multi::{many0, many1, separated_list0};
use nom::sequence::{delimited, pair, preceded, tuple};

use crate::{
    filter, identifier, is_ws, keyword, not_ws, skip_till, str_lit, ws, ErrorContext, Expr, Filter,
    ParseResult, State, Target, WithSpan,
};

#[derive(Debug, PartialEq)]
pub enum Node<'a> {
    Lit(WithSpan<'a, Lit<'a>>),
    Comment(WithSpan<'a, Comment<'a>>),
    Expr(Ws, WithSpan<'a, Expr<'a>>),
    Call(WithSpan<'a, Call<'a>>),
    Let(WithSpan<'a, Let<'a>>),
    If(WithSpan<'a, If<'a>>),
    Match(WithSpan<'a, Match<'a>>),
    Loop(Box<WithSpan<'a, Loop<'a>>>),
    Extends(WithSpan<'a, Extends<'a>>),
    BlockDef(WithSpan<'a, BlockDef<'a>>),
    Include(WithSpan<'a, Include<'a>>),
    Import(WithSpan<'a, Import<'a>>),
    Macro(WithSpan<'a, Macro<'a>>),
    Raw(WithSpan<'a, Raw<'a>>),
    Break(WithSpan<'a, Ws>),
    Continue(WithSpan<'a, Ws>),
    FilterBlock(WithSpan<'a, FilterBlock<'a>>),
}

impl<'a> Node<'a> {
    pub(super) fn many(i: &'a str, s: &State<'_>) -> ParseResult<'a, Vec<Self>> {
        complete(many0(alt((
            map(|i| Lit::parse(i, s), Self::Lit),
            map(|i| Comment::parse(i, s), Self::Comment),
            |i| Self::expr(i, s),
            |i| Self::parse(i, s),
        ))))(i)
    }

    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        #[inline]
        fn wrap<'a, T>(
            func: impl FnOnce(T) -> Node<'a>,
            result: ParseResult<'a, T>,
        ) -> ParseResult<'a, Node<'a>> {
            result.map(|(i, n)| (i, func(n)))
        }

        let start = i;
        let (j, tag) = preceded(
            |i| s.tag_block_start(i),
            peek(preceded(
                pair(opt(Whitespace::parse), take_till(not_ws)),
                identifier,
            )),
        )(i)?;

        let func = match tag {
            "call" => |i, s| wrap(Self::Call, Call::parse(i, s)),
            "let" | "set" => |i, s| wrap(Self::Let, Let::parse(i, s)),
            "if" => |i, s| wrap(Self::If, If::parse(i, s)),
            "for" => |i, s| wrap(|n| Self::Loop(Box::new(n)), Loop::parse(i, s)),
            "match" => |i, s| wrap(Self::Match, Match::parse(i, s)),
            "extends" => |i, _s| wrap(Self::Extends, Extends::parse(i)),
            "include" => |i, _s| wrap(Self::Include, Include::parse(i)),
            "import" => |i, _s| wrap(Self::Import, Import::parse(i)),
            "block" => |i, s| wrap(Self::BlockDef, BlockDef::parse(i, s)),
            "macro" => |i, s| wrap(Self::Macro, Macro::parse(i, s)),
            "raw" => |i, s| wrap(Self::Raw, Raw::parse(i, s)),
            "break" => |i, s| Self::r#break(i, s),
            "continue" => |i, s| Self::r#continue(i, s),
            "filter" => |i, s| wrap(Self::FilterBlock, FilterBlock::parse(i, s)),
            _ => {
                return Err(ErrorContext::from_err(nom::Err::Error(error_position!(
                    i,
                    ErrorKind::Tag
                ))));
            }
        };

        let (i, node) = s.nest(j, |i| func(i, s))?;

        let (i, closed) = cut(alt((
            value(true, |i| s.tag_block_end(i)),
            value(false, ws(eof)),
        )))(i)?;
        match closed {
            true => Ok((i, node)),
            false => Err(ErrorContext::unclosed("block", s.syntax.block_end, start).into()),
        }
    }

    fn r#break(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("break")),
            opt(Whitespace::parse),
        ));
        let (j, (pws, _, nws)) = p(i)?;
        if !s.is_in_loop() {
            return Err(nom::Err::Failure(ErrorContext::new(
                "you can only `break` inside a `for` loop",
                i,
            )));
        }
        Ok((j, Self::Break(WithSpan::new(Ws(pws, nws), i))))
    }

    fn r#continue(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("continue")),
            opt(Whitespace::parse),
        ));
        let (j, (pws, _, nws)) = p(i)?;
        if !s.is_in_loop() {
            return Err(nom::Err::Failure(ErrorContext::new(
                "you can only `continue` inside a `for` loop",
                i,
            )));
        }
        Ok((j, Self::Continue(WithSpan::new(Ws(pws, nws), i))))
    }

    fn expr(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        let start = i;
        let (i, (pws, expr)) = preceded(
            |i| s.tag_expr_start(i),
            cut(pair(
                opt(Whitespace::parse),
                ws(|i| Expr::parse(i, s.level.get())),
            )),
        )(i)?;

        let (i, (nws, closed)) = cut(pair(
            opt(Whitespace::parse),
            alt((value(true, |i| s.tag_expr_end(i)), value(false, ws(eof)))),
        ))(i)?;
        match closed {
            true => Ok((i, Self::Expr(Ws(pws, nws), expr))),
            false => Err(ErrorContext::unclosed("expression", s.syntax.expr_end, start).into()),
        }
    }

    pub fn span(&self) -> &str {
        match self {
            Self::Lit(span) => span.span,
            Self::Comment(span) => span.span,
            Self::Expr(_, span) => span.span,
            Self::Call(span) => span.span,
            Self::Let(span) => span.span,
            Self::If(span) => span.span,
            Self::Match(span) => span.span,
            Self::Loop(span) => span.span,
            Self::Extends(span) => span.span,
            Self::BlockDef(span) => span.span,
            Self::Include(span) => span.span,
            Self::Import(span) => span.span,
            Self::Macro(span) => span.span,
            Self::Raw(span) => span.span,
            Self::Break(span) => span.span,
            Self::Continue(span) => span.span,
            Self::FilterBlock(span) => span.span,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct When<'a> {
    pub ws: Ws,
    pub target: Target<'a>,
    pub nodes: Vec<Node<'a>>,
}

impl<'a> When<'a> {
    fn r#match(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            |i| s.tag_block_start(i),
            opt(Whitespace::parse),
            ws(keyword("else")),
            cut(tuple((
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
                cut(|i| Node::many(i, s)),
            ))),
        ));
        let (i, (_, pws, _, (nws, _, nodes))) = p(i)?;
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    target: Target::Placeholder("_"),
                    nodes,
                },
                start,
            ),
        ))
    }

    #[allow(clippy::self_named_constructors)]
    fn when(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            |i| s.tag_block_start(i),
            opt(Whitespace::parse),
            ws(keyword("when")),
            cut(tuple((
                ws(|i| Target::parse(i, s)),
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
                cut(|i| Node::many(i, s)),
            ))),
        ));
        let (i, (_, pws, _, (target, nws, _, nodes))) = p(i)?;
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    target,
                    nodes,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Cond<'a> {
    pub ws: Ws,
    pub cond: Option<CondTest<'a>>,
    pub nodes: Vec<Node<'a>>,
}

impl<'a> Cond<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let (i, (_, pws, cond, nws, _, nodes)) = tuple((
            |i| s.tag_block_start(i),
            opt(Whitespace::parse),
            alt((
                preceded(ws(keyword("else")), opt(|i| CondTest::parse(i, s))),
                preceded(
                    ws(keyword("elif")),
                    cut(map(|i| CondTest::parse_cond(i, s), Some)),
                ),
            )),
            opt(Whitespace::parse),
            cut(|i| s.tag_block_end(i)),
            cut(|i| Node::many(i, s)),
        ))(i)?;
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    cond,
                    nodes,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct CondTest<'a> {
    pub target: Option<Target<'a>>,
    pub expr: WithSpan<'a, Expr<'a>>,
}

impl<'a> CondTest<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        preceded(ws(keyword("if")), cut(|i| Self::parse_cond(i, s)))(i)
    }

    fn parse_cond(i: &'a str, s: &State<'_>) -> ParseResult<'a, Self> {
        let (i, (target, expr)) = pair(
            opt(delimited(
                ws(alt((keyword("let"), keyword("set")))),
                ws(|i| Target::parse(i, s)),
                ws(char('=')),
            )),
            ws(|i| Expr::parse(i, s.level.get())),
        )(i)?;
        Ok((i, Self { target, expr }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Whitespace {
    Preserve,
    Suppress,
    Minimize,
}

impl Whitespace {
    fn parse(i: &str) -> ParseResult<'_, Self> {
        alt((
            value(Self::Preserve, char('+')),
            value(Self::Suppress, char('-')),
            value(Self::Minimize, char('~')),
        ))(i)
    }
}

#[derive(Debug, PartialEq)]
pub struct Loop<'a> {
    pub ws1: Ws,
    pub var: Target<'a>,
    pub iter: WithSpan<'a, Expr<'a>>,
    pub cond: Option<WithSpan<'a, Expr<'a>>>,
    pub body: Vec<Node<'a>>,
    pub ws2: Ws,
    pub else_nodes: Vec<Node<'a>>,
    pub ws3: Ws,
}

impl<'a> Loop<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        fn content<'a>(i: &'a str, s: &State<'_>) -> ParseResult<'a, Vec<Node<'a>>> {
            s.enter_loop();
            let result = Node::many(i, s);
            s.leave_loop();
            result
        }

        let start = i;
        let if_cond = preceded(
            ws(keyword("if")),
            cut(ws(|i| Expr::parse(i, s.level.get()))),
        );

        let else_block = |i| {
            let mut p = preceded(
                ws(keyword("else")),
                cut(tuple((
                    opt(Whitespace::parse),
                    delimited(
                        |i| s.tag_block_end(i),
                        |i| Node::many(i, s),
                        |i| s.tag_block_start(i),
                    ),
                    opt(Whitespace::parse),
                ))),
            );
            let (i, (pws, nodes, nws)) = p(i)?;
            Ok((i, (pws, nodes, nws)))
        };

        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("for")),
            cut(tuple((
                ws(|i| Target::parse(i, s)),
                ws(keyword("in")),
                cut(tuple((
                    ws(|i| Expr::parse(i, s.level.get())),
                    opt(if_cond),
                    opt(Whitespace::parse),
                    |i| s.tag_block_end(i),
                    cut(tuple((
                        |i| content(i, s),
                        cut(tuple((
                            |i| s.tag_block_start(i),
                            opt(Whitespace::parse),
                            opt(else_block),
                            ws(keyword("endfor")),
                            opt(Whitespace::parse),
                        ))),
                    ))),
                ))),
            ))),
        ));
        let (i, (pws1, _, (var, _, (iter, cond, nws1, _, (body, (_, pws2, else_block, _, nws2)))))) =
            p(i)?;
        let (nws3, else_block, pws3) = else_block.unwrap_or_default();
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws1: Ws(pws1, nws1),
                    var,
                    iter,
                    cond,
                    body,
                    ws2: Ws(pws2, nws3),
                    else_nodes: else_block,
                    ws3: Ws(pws3, nws2),
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Macro<'a> {
    pub ws1: Ws,
    pub name: &'a str,
    pub args: Vec<&'a str>,
    pub nodes: Vec<Node<'a>>,
    pub ws2: Ws,
}

impl<'a> Macro<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        fn parameters(i: &str) -> ParseResult<'_, Vec<&str>> {
            delimited(
                ws(char('(')),
                separated_list0(char(','), ws(identifier)),
                tuple((opt(ws(char(','))), char(')'))),
            )(i)
        }

        let start_s = i;
        let mut start = tuple((
            opt(Whitespace::parse),
            ws(keyword("macro")),
            cut(tuple((
                ws(identifier),
                opt(ws(parameters)),
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
            ))),
        ));
        let (j, (pws1, _, (name, params, nws1, _))) = start(i)?;
        if is_rust_keyword(name) {
            return Err(nom::Err::Failure(ErrorContext::new(
                format!("'{name}' is not a valid name for a macro"),
                i,
            )));
        }

        let mut end = cut(tuple((
            |i| Node::many(i, s),
            cut(tuple((
                |i| s.tag_block_start(i),
                opt(Whitespace::parse),
                ws(keyword("endmacro")),
                cut(preceded(
                    opt(|before| {
                        let (after, end_name) = ws(identifier)(before)?;
                        check_end_name(before, after, name, end_name, "macro")
                    }),
                    opt(Whitespace::parse),
                )),
            ))),
        )));
        let (i, (contents, (_, pws2, _, nws2))) = end(j)?;

        Ok((
            i,
            WithSpan::new(
                Self {
                    ws1: Ws(pws1, nws1),
                    name,
                    args: params.unwrap_or_default(),
                    nodes: contents,
                    ws2: Ws(pws2, nws2),
                },
                start_s,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct FilterBlock<'a> {
    pub ws1: Ws,
    pub filters: Filter<'a>,
    pub nodes: Vec<Node<'a>>,
    pub ws2: Ws,
}

impl<'a> FilterBlock<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let mut level = s.level.get();
        let start_s = i;
        let mut start = tuple((
            opt(Whitespace::parse),
            ws(keyword("filter")),
            cut(tuple((
                ws(identifier),
                opt(|i| Expr::arguments(i, s.level.get(), false)),
                many0(|i| filter(i, &mut level).map(|(j, (name, params))| (j, (name, params, i)))),
                ws(|i| Ok((i, ()))),
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
            ))),
        ));
        let (i, (pws1, _, (filter_name, params, extra_filters, _, nws1, _))) = start(i)?;

        let mut arguments = params.unwrap_or_default();
        arguments.insert(0, WithSpan::new(Expr::FilterSource, start_s));
        let mut filters = Filter {
            name: filter_name,
            arguments,
        };
        for (filter_name, args, span) in extra_filters {
            filters = Filter {
                name: filter_name,
                arguments: {
                    let mut args = args.unwrap_or_default();
                    args.insert(0, WithSpan::new(Expr::Filter(filters), span));
                    args
                },
            };
        }

        let mut end = cut(tuple((
            |i| Node::many(i, s),
            cut(tuple((
                |i| s.tag_block_start(i),
                opt(Whitespace::parse),
                ws(keyword("endfilter")),
                opt(Whitespace::parse),
            ))),
        )));
        let (i, (nodes, (_, pws2, _, nws2))) = end(i)?;

        Ok((
            i,
            WithSpan::new(
                Self {
                    ws1: Ws(pws1, nws1),
                    filters,
                    nodes,
                    ws2: Ws(pws2, nws2),
                },
                start_s,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Import<'a> {
    pub ws: Ws,
    pub path: &'a str,
    pub scope: &'a str,
}

impl<'a> Import<'a> {
    fn parse(i: &'a str) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("import")),
            cut(tuple((
                ws(str_lit),
                ws(keyword("as")),
                cut(pair(ws(identifier), opt(Whitespace::parse))),
            ))),
        ));
        let (i, (pws, _, (path, _, (scope, nws)))) = p(i)?;
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    path,
                    scope,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Call<'a> {
    pub ws: Ws,
    pub scope: Option<&'a str>,
    pub name: &'a str,
    pub args: Vec<WithSpan<'a, Expr<'a>>>,
}

impl<'a> Call<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("call")),
            cut(tuple((
                opt(tuple((ws(identifier), ws(tag("::"))))),
                ws(identifier),
                opt(ws(|nested| Expr::arguments(nested, s.level.get(), true))),
                opt(Whitespace::parse),
            ))),
        ));
        let (i, (pws, _, (scope, name, args, nws))) = p(i)?;
        let scope = scope.map(|(scope, _)| scope);
        let args = args.unwrap_or_default();
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    scope,
                    name,
                    args,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Match<'a> {
    pub ws1: Ws,
    pub expr: WithSpan<'a, Expr<'a>>,
    pub arms: Vec<WithSpan<'a, When<'a>>>,
    pub ws2: Ws,
}

impl<'a> Match<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("match")),
            cut(tuple((
                ws(|i| Expr::parse(i, s.level.get())),
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
                cut(tuple((
                    ws(many0(ws(value((), |i| Comment::parse(i, s))))),
                    many1(|i| When::when(i, s)),
                    cut(tuple((
                        opt(|i| When::r#match(i, s)),
                        cut(tuple((
                            ws(|i| s.tag_block_start(i)),
                            opt(Whitespace::parse),
                            ws(keyword("endmatch")),
                            opt(Whitespace::parse),
                        ))),
                    ))),
                ))),
            ))),
        ));
        let (i, (pws1, _, (expr, nws1, _, (_, arms, (else_arm, (_, pws2, _, nws2)))))) = p(i)?;

        let mut arms = arms;
        if let Some(arm) = else_arm {
            arms.push(arm);
        }

        Ok((
            i,
            WithSpan::new(
                Self {
                    ws1: Ws(pws1, nws1),
                    expr,
                    arms,
                    ws2: Ws(pws2, nws2),
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct BlockDef<'a> {
    pub ws1: Ws,
    pub name: &'a str,
    pub nodes: Vec<Node<'a>>,
    pub ws2: Ws,
}

impl<'a> BlockDef<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start_s = i;
        let mut start = tuple((
            opt(Whitespace::parse),
            ws(keyword("block")),
            cut(tuple((ws(identifier), opt(Whitespace::parse), |i| {
                s.tag_block_end(i)
            }))),
        ));
        let (i, (pws1, _, (name, nws1, _))) = start(i)?;

        let mut end = cut(tuple((
            |i| Node::many(i, s),
            cut(tuple((
                |i| s.tag_block_start(i),
                opt(Whitespace::parse),
                ws(keyword("endblock")),
                cut(tuple((
                    opt(|before| {
                        let (after, end_name) = ws(identifier)(before)?;
                        check_end_name(before, after, name, end_name, "block")
                    }),
                    opt(Whitespace::parse),
                ))),
            ))),
        )));
        let (i, (nodes, (_, pws2, _, (_, nws2)))) = end(i)?;

        Ok((
            i,
            WithSpan::new(
                BlockDef {
                    ws1: Ws(pws1, nws1),
                    name,
                    nodes,
                    ws2: Ws(pws2, nws2),
                },
                start_s,
            ),
        ))
    }
}

fn check_end_name<'a>(
    before: &'a str,
    after: &'a str,
    name: &'a str,
    end_name: &'a str,
    kind: &str,
) -> ParseResult<'a> {
    if name == end_name {
        return Ok((after, end_name));
    }

    Err(nom::Err::Failure(ErrorContext::new(
        match name.is_empty() && !end_name.is_empty() {
            true => format!("unexpected name `{end_name}` in `end{kind}` tag for unnamed `{kind}`"),
            false => format!("expected name `{name}` in `end{kind}` tag, found `{end_name}`"),
        },
        before,
    )))
}

#[derive(Debug, PartialEq)]
pub struct Lit<'a> {
    pub lws: &'a str,
    pub val: &'a str,
    pub rws: &'a str,
}

impl<'a> Lit<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let p_start = alt((
            tag(s.syntax.block_start),
            tag(s.syntax.comment_start),
            tag(s.syntax.expr_start),
        ));

        let (i, _) = not(eof)(i)?;
        let (i, content) = opt(recognize(skip_till(p_start)))(i)?;
        let (i, content) = match content {
            Some("") => {
                // {block,comment,expr}_start follows immediately.
                return Err(nom::Err::Error(error_position!(i, ErrorKind::TakeUntil)));
            }
            Some(content) => (i, content),
            None => ("", i), // there is no {block,comment,expr}_start: take everything
        };
        Ok((i, WithSpan::new(Self::split_ws_parts(content), start)))
    }

    pub(crate) fn split_ws_parts(s: &'a str) -> Self {
        let trimmed_start = s.trim_start_matches(is_ws);
        let len_start = s.len() - trimmed_start.len();
        let trimmed = trimmed_start.trim_end_matches(is_ws);
        Self {
            lws: &s[..len_start],
            val: trimmed,
            rws: &trimmed_start[trimmed.len()..],
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Raw<'a> {
    pub ws1: Ws,
    pub lit: Lit<'a>,
    pub ws2: Ws,
}

impl<'a> Raw<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let endraw = tuple((
            |i| s.tag_block_start(i),
            opt(Whitespace::parse),
            ws(keyword("endraw")),
            opt(Whitespace::parse),
            peek(|i| s.tag_block_end(i)),
        ));

        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("raw")),
            cut(tuple((
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
                consumed(skip_till(endraw)),
            ))),
        ));

        let (_, (pws1, _, (nws1, _, (contents, (i, (_, pws2, _, nws2, _)))))) = p(i)?;
        let lit = Lit::split_ws_parts(contents);
        let ws1 = Ws(pws1, nws1);
        let ws2 = Ws(pws2, nws2);
        Ok((i, WithSpan::new(Self { ws1, lit, ws2 }, start)))
    }
}

#[derive(Debug, PartialEq)]
pub struct Let<'a> {
    pub ws: Ws,
    pub var: Target<'a>,
    pub val: Option<WithSpan<'a, Expr<'a>>>,
}

impl<'a> Let<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(alt((keyword("let"), keyword("set")))),
            cut(tuple((
                ws(|i| Target::parse(i, s)),
                opt(preceded(
                    ws(char('=')),
                    ws(|i| Expr::parse(i, s.level.get())),
                )),
                opt(Whitespace::parse),
            ))),
        ));
        let (i, (pws, _, (var, val, nws))) = p(i)?;

        Ok((
            i,
            WithSpan::new(
                Let {
                    ws: Ws(pws, nws),
                    var,
                    val,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct If<'a> {
    pub ws: Ws,
    pub branches: Vec<WithSpan<'a, Cond<'a>>>,
}

impl<'a> If<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            |i| CondTest::parse(i, s),
            cut(tuple((
                opt(Whitespace::parse),
                |i| s.tag_block_end(i),
                cut(tuple((
                    |i| Node::many(i, s),
                    many0(|i| Cond::parse(i, s)),
                    cut(tuple((
                        |i| s.tag_block_start(i),
                        opt(Whitespace::parse),
                        ws(keyword("endif")),
                        opt(Whitespace::parse),
                    ))),
                ))),
            ))),
        ));

        let (i, (pws1, cond, (nws1, _, (nodes, elifs, (_, pws2, _, nws2))))) = p(i)?;
        let mut branches = vec![WithSpan::new(
            Cond {
                ws: Ws(pws1, nws1),
                cond: Some(cond),
                nodes,
            },
            start,
        )];
        branches.extend(elifs);

        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws2, nws2),
                    branches,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Include<'a> {
    pub ws: Ws,
    pub path: &'a str,
}

impl<'a> Include<'a> {
    fn parse(i: &'a str) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;
        let mut p = tuple((
            opt(Whitespace::parse),
            ws(keyword("include")),
            cut(pair(ws(str_lit), opt(Whitespace::parse))),
        ));
        let (i, (pws, _, (path, nws))) = p(i)?;
        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    path,
                },
                start,
            ),
        ))
    }
}

#[derive(Debug, PartialEq)]
pub struct Extends<'a> {
    pub path: &'a str,
}

impl<'a> Extends<'a> {
    fn parse(i: &'a str) -> ParseResult<'a, WithSpan<'a, Self>> {
        let start = i;

        let (i, (pws, _, (path, nws))) = tuple((
            opt(Whitespace::parse),
            ws(keyword("extends")),
            cut(pair(ws(str_lit), opt(Whitespace::parse))),
        ))(i)?;
        match (pws, nws) {
            (None, None) => Ok((i, WithSpan::new(Self { path }, start))),
            (_, _) => Err(nom::Err::Failure(ErrorContext::new(
                "whitespace control is not allowed on `extends`",
                start,
            ))),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Comment<'a> {
    pub ws: Ws,
    pub content: &'a str,
}

impl<'a> Comment<'a> {
    fn parse(i: &'a str, s: &State<'_>) -> ParseResult<'a, WithSpan<'a, Self>> {
        #[derive(Debug, Clone, Copy)]
        enum Tag {
            Open,
            Close,
        }

        fn tag<'a>(i: &'a str, s: &State<'_>) -> ParseResult<'a, Tag> {
            alt((
                value(Tag::Open, |i| s.tag_comment_start(i)),
                value(Tag::Close, |i| s.tag_comment_end(i)),
            ))(i)
        }

        fn content<'a>(mut i: &'a str, s: &State<'_>) -> ParseResult<'a, ()> {
            let mut depth = 0usize;
            loop {
                let start = i;
                let (_, tag) = opt(skip_till(|i| tag(i, s)))(i)?;
                let Some((j, tag)) = tag else {
                    return Err(
                        ErrorContext::unclosed("comment", s.syntax.comment_end, start).into(),
                    );
                };
                match tag {
                    Tag::Open => match depth.checked_add(1) {
                        Some(new_depth) => depth = new_depth,
                        None => {
                            return Err(nom::Err::Failure(ErrorContext::new(
                                "too deeply nested comments",
                                start,
                            )));
                        }
                    },
                    Tag::Close => match depth.checked_sub(1) {
                        Some(new_depth) => depth = new_depth,
                        None => return Ok((j, ())),
                    },
                }
                i = j;
            }
        }

        let start = i;
        let (i, (pws, content)) = pair(
            preceded(|i| s.tag_comment_start(i), opt(Whitespace::parse)),
            recognize(cut(|i| content(i, s))),
        )(i)?;

        let mut nws = None;
        if let Some(content) = content.strip_suffix(s.syntax.comment_end) {
            nws = match content.chars().last() {
                Some('-') => Some(Whitespace::Suppress),
                Some('+') => Some(Whitespace::Preserve),
                Some('~') => Some(Whitespace::Minimize),
                _ => None,
            }
        };

        Ok((
            i,
            WithSpan::new(
                Self {
                    ws: Ws(pws, nws),
                    content,
                },
                start,
            ),
        ))
    }
}

/// First field is "minus/plus sign was used on the left part of the item".
///
/// Second field is "minus/plus sign was used on the right part of the item".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ws(pub Option<Whitespace>, pub Option<Whitespace>);

#[doc(hidden)]
pub const MAX_KW_LEN: usize = 8;
const MAX_REPL_LEN: usize = MAX_KW_LEN + 2;
#[doc(hidden)]
pub const KWS: &[&[[u8; MAX_REPL_LEN]]] = {
    // FIXME: Replace `u8` with `[core:ascii::Char; MAX_REPL_LEN]` once
    //        <https://github.com/rust-lang/rust/issues/110998> is stable.

    const KW2: &[[u8; MAX_REPL_LEN]] = &[
        *b"r#as______",
        *b"r#do______",
        *b"r#fn______",
        *b"r#if______",
        *b"r#in______",
    ];
    const KW3: &[[u8; MAX_REPL_LEN]] = &[
        *b"r#box_____",
        *b"r#dyn_____",
        *b"r#for_____",
        *b"r#let_____",
        *b"r#mod_____",
        *b"r#mut_____",
        *b"r#pub_____",
        *b"r#ref_____",
        *b"r#try_____",
        *b"r#use_____",
    ];
    const KW4: &[[u8; MAX_REPL_LEN]] = &[
        *b"r#else____",
        *b"r#enum____",
        *b"r#impl____",
        *b"r#move____",
        *b"r#priv____",
        *b"r#true____",
        *b"r#type____",
    ];
    const KW5: &[[u8; MAX_REPL_LEN]] = &[
        *b"r#async___",
        *b"r#await___",
        *b"r#break___",
        *b"r#const___",
        *b"r#crate___",
        *b"r#false___",
        *b"r#final___",
        *b"r#macro___",
        *b"r#match___",
        *b"r#trait___",
        *b"r#where___",
        *b"r#while___",
        *b"r#yield___",
    ];
    const KW6: &[[u8; MAX_REPL_LEN]] = &[
        *b"r#become__",
        *b"r#extern__",
        *b"r#return__",
        *b"r#static__",
        *b"r#struct__",
        *b"r#typeof__",
        *b"r#unsafe__",
    ];
    const KW7: &[[u8; MAX_REPL_LEN]] = &[*b"r#unsized_", *b"r#virtual_"];
    const KW8: &[[u8; MAX_REPL_LEN]] = &[*b"r#abstract", *b"r#continue", *b"r#override"];

    &[&[], &[], KW2, KW3, KW4, KW5, KW6, KW7, KW8]
};

// These ones are only used in the parser, hence why they're private.
const KWS_EXTRA: &[&[[u8; MAX_REPL_LEN]]] = {
    const KW4: &[[u8; MAX_REPL_LEN]] = &[*b"r#loop____", *b"r#self____", *b"r#Self____"];
    const KW5: &[[u8; MAX_REPL_LEN]] = &[*b"r#super___", *b"r#union___"];

    &[&[], &[], &[], &[], KW4, KW5, &[], &[], &[]]
};

fn is_rust_keyword(ident: &str) -> bool {
    fn is_rust_keyword_inner(
        kws: &[&[[u8; MAX_REPL_LEN]]],
        padded_ident: &[u8; MAX_KW_LEN],
        ident_len: usize,
    ) -> bool {
        // Since the individual buckets are quite short, a linear search is faster than a binary search.
        kws[ident_len]
            .iter()
            .any(|&probe| padded_ident == &probe[2..])
    }
    if ident.len() > MAX_KW_LEN {
        return false;
    }
    let ident_len = ident.len();

    let mut padded_ident = [b'_'; MAX_KW_LEN];
    padded_ident[..ident.len()].copy_from_slice(ident.as_bytes());

    is_rust_keyword_inner(KWS, &padded_ident, ident_len)
        || is_rust_keyword_inner(KWS_EXTRA, &padded_ident, ident_len)
}

#[cfg(test)]
mod kws_tests {
    use super::{is_rust_keyword, KWS, KWS_EXTRA, MAX_REPL_LEN};

    fn ensure_utf8_inner(entry: &[&[[u8; MAX_REPL_LEN]]]) {
        for kws in entry {
            for kw in *kws {
                if std::str::from_utf8(kw).is_err() {
                    panic!("not UTF-8: {:?}", kw);
                }
            }
        }
    }

    // Ensure that all strings are UTF-8, because we use `from_utf8_unchecked()` further down.
    #[test]
    fn ensure_utf8() {
        assert_eq!(KWS.len(), KWS_EXTRA.len());
        ensure_utf8_inner(KWS);
        ensure_utf8_inner(KWS_EXTRA);
    }

    #[test]
    fn test_is_rust_keyword() {
        assert!(is_rust_keyword("super"));
        assert!(is_rust_keyword("become"));
        assert!(!is_rust_keyword("supeeeer"));
        assert!(!is_rust_keyword("sur"));
    }
}
