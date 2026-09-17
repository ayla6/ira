# Age-rating marks

Official rating icons vendored from [Wikimedia Commons](https://commons.wikimedia.org),
where they are freely licensed (public-domain text-logos / simple geometry unless a
file's page states otherwise — several are CC BY from their board's publications).
The boards themselves hold trademark rights in the marks; use here is nominative, to
identify each rating the way the sources report it.

Dropping a new `<slug>.svg` in here needs a matching arm in
`crates/images/src/ratings.rs` (`slug` = board id + value, lowercased, punctuation
stripped — `ira_models::ratings::rating_slug` computes it).

| Asset | Commons file the copy came from |
|---|---|
| `acb-g.svg` | [File:Australian Classification General (G).svg](https://commons.wikimedia.org/wiki/File:Australian%20Classification%20General%20%28G%29.svg) |
| `acb-m.svg` | [File:Australian Classification Mature (M).svg](https://commons.wikimedia.org/wiki/File:Australian%20Classification%20Mature%20%28M%29.svg) |
| `acb-pg.svg` | [File:Australian Classification Parental Guidance (PG).svg](https://commons.wikimedia.org/wiki/File:Australian%20Classification%20Parental%20Guidance%20%28PG%29.svg) |
| `bbfc-12.svg` | [File:BBFC 12 2019.svg](https://commons.wikimedia.org/wiki/File:BBFC%2012%202019.svg) |
| `bbfc-15.svg` | [File:BBFC 15.svg](https://commons.wikimedia.org/wiki/File:BBFC%2015.svg) |
| `bbfc-18.svg` | [File:BBFC 18 2019.svg](https://commons.wikimedia.org/wiki/File:BBFC%2018%202019.svg) |
| `bbfc-pg.svg` | [File:BBFC PG 2019.svg](https://commons.wikimedia.org/wiki/File:BBFC%20PG%202019.svg) |
| `bbfc-u.svg` | [File:BBFC U 2002.svg](https://commons.wikimedia.org/wiki/File:BBFC%20U%202002.svg) |
| `classind-10.svg` | [File:Classificação Indicativa 10 anos.svg](https://commons.wikimedia.org/wiki/File:Classifica%C3%A7%C3%A3o%20Indicativa%2010%20anos.svg) |
| `classind-14.svg` | [File:Classificação Indicativa 14 anos.svg](https://commons.wikimedia.org/wiki/File:Classifica%C3%A7%C3%A3o%20Indicativa%2014%20anos.svg) |
| `classind-16.svg` | [File:Classificação Indicativa 16 anos.svg](https://commons.wikimedia.org/wiki/File:Classifica%C3%A7%C3%A3o%20Indicativa%2016%20anos.svg) |
| `classind-l.svg` | [File:Classificação Indicativa Livre.svg](https://commons.wikimedia.org/wiki/File:Classifica%C3%A7%C3%A3o%20Indicativa%20Livre.svg) |
| `classinda-18.svg` | [File:Autoclassificação Indicativa 18 anos.svg](https://commons.wikimedia.org/wiki/File:Autoclassifica%C3%A7%C3%A3o%20Indicativa%2018%20anos.svg) |
| `classinda-al.svg` | [File:Autoclassificação Indicativa Livre.svg](https://commons.wikimedia.org/wiki/File:Autoclassifica%C3%A7%C3%A3o%20Indicativa%20Livre.svg) |
| `csrr-0.svg` | [File:GSRR G logo.svg](https://commons.wikimedia.org/wiki/File:GSRR%20G%20logo.svg) |
| `csrr-12.svg` | [File:GSRR PG 12 logo.svg](https://commons.wikimedia.org/wiki/File:GSRR%20PG%2012%20logo.svg) |
| `csrr-15.svg` | [File:GSRR PG 15 logo.svg](https://commons.wikimedia.org/wiki/File:GSRR%20PG%2015%20logo.svg) |
| `csrr-18.svg` | [File:GSRR R logo.svg](https://commons.wikimedia.org/wiki/File:GSRR%20R%20logo.svg) |
| `csrr-6.svg` | [File:GSRR P logo.svg](https://commons.wikimedia.org/wiki/File:GSRR%20P%20logo.svg) |
| `elspa-11.svg` | [File:ELSPA 11-14.svg](https://commons.wikimedia.org/wiki/File:ELSPA%2011-14.svg) |
| `elspa-15.svg` | [File:ELSPA 15-17.svg](https://commons.wikimedia.org/wiki/File:ELSPA%2015-17.svg) |
| `elspa-18.svg` | [File:ELSPA 18+.svg](https://commons.wikimedia.org/wiki/File:ELSPA%2018%2B.svg) |
| `elspa-3.svg` | [File:ELSPA 3-10.svg](https://commons.wikimedia.org/wiki/File:ELSPA%203-10.svg) |
| `esrb-rp.svg` | [File:ESRB RP.svg](https://commons.wikimedia.org/wiki/File:ESRB%20RP.svg) |
| `grb-19.svg` | [File:KMRB 19 (2024).svg](https://commons.wikimedia.org/wiki/File:KMRB%2019%20%282024%29.svg) |
| `igrs-13.svg` | [File:IGRS 13+ 2024.svg](https://commons.wikimedia.org/wiki/File:IGRS%2013%2B%202024.svg) |
| `igrs-15.svg` | [File:IGRS 15+ 2024.svg](https://commons.wikimedia.org/wiki/File:IGRS%2015%2B%202024.svg) |
| `igrs-18.svg` | [File:IGRS 18+ 2024.svg](https://commons.wikimedia.org/wiki/File:IGRS%2018%2B%202024.svg) |
| `igrs-rc.svg` | [File:IGRS RC 2024.svg](https://commons.wikimedia.org/wiki/File:IGRS%20RC%202024.svg) |
| `igrs-su.svg` | [File:IGRS SU.svg](https://commons.wikimedia.org/wiki/File:IGRS%20SU.svg) |
| `nzoflc-g.svg` | [File:OFLC G label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20G%20label%20%282022%29.svg) |
| `nzoflc-m.svg` | [File:OFLC M label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20M%20label%20%282022%29.svg) |
| `nzoflc-pg.svg` | [File:OFLC PG label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20PG%20label%20%282022%29.svg) |
| `nzoflc-r13.svg` | [File:OFLC Restricted 13 (R13) label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20Restricted%2013%20%28R13%29%20label%20%282022%29.svg) |
| `nzoflc-r15.svg` | [File:OFLC Restricted 15 (R15) label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20Restricted%2015%20%28R15%29%20label%20%282022%29.svg) |
| `nzoflc-r16.svg` | [File:OFLC Restricted 16 (R16) label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20Restricted%2016%20%28R16%29%20label%20%282022%29.svg) |
| `nzoflc-r18.svg` | [File:OFLC Restricted 18 (R18) label (2022).svg](https://commons.wikimedia.org/wiki/File:OFLC%20Restricted%2018%20%28R18%29%20label%20%282022%29.svg) |
| `sell-12.svg` | [File:GSRR PG 12 (IARC).svg](https://commons.wikimedia.org/wiki/File:GSRR%20PG%2012%20%28IARC%29.svg) |
