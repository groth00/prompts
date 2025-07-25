SELECT
  base.t as base,
  c1.t AS c1,
  c2.t AS c2,
  c3.t AS c3,
  c4.t AS c4,
  c5.t AS c5,
  c6.t AS c6
FROM
  templates
LEFT JOIN base ON templates.base = base.id
LEFT JOIN characters AS c1 ON templates.c1 = c1.id
LEFT JOIN characters AS c2 ON templates.c2 = c2.id
LEFT JOIN characters AS c3 ON templates.c3 = c3.id
LEFT JOIN characters AS c4 ON templates.c4 = c4.id
LEFT JOIN characters AS c5 ON templates.c5 = c5.id
LEFT JOIN characters AS c6 ON templates.c6 = c6.id
WHERE templates.name = ?1;
