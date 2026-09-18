;; Small shared text renderer: role labels, bold, inline code, and command tokens.
;; Styling uses syntax accents, since ui.text.focus may equal ordinary text.
(require-builtin helix/components as ui.)
(provide rich-lines rich-draw)

(define (starts? text prefix)
  (and (>= (string-length text) (string-length prefix))
       (equal? (substring text 0 (string-length prefix)) prefix)))
(define (paint chars kind) (map (lambda (c) (cons c kind)) chars))
(define (letter? c)
  (or (and (char>=? c #\a) (char<=? c #\z))
      (and (char>=? c #\A) (char<=? c #\Z))))
(define (digit? c) (and (char>=? c #\0) (char<=? c #\9)))
(define (word-char? c)
  (or (letter? c) (digit? c) (member c '(#\- #\_ #\/ #\:))))
(define (inline chars)
  (cond [(null? chars) (list)]
        [(char=? (car chars) #\`)
         (let find ([rest (cdr chars)] [count 0])
           (cond [(null? rest) (cons (cons (car chars) 'text) (inline (cdr chars)))]
                 [(char=? (car rest) #\`)
                  (append (paint (take (cdr chars) count) 'code) (inline (cdr rest)))]
                 [else (find (cdr rest) (+ count 1))]))]
        [(and (member (car chars) '(#\/ #\:))
              (not (null? (cdr chars))) (letter? (cadr chars)))
         (let find ([rest chars] [count 0])
           (if (and (not (null? rest)) (word-char? (car rest)))
               (find (cdr rest) (+ count 1))
               (append (paint (take chars count) 'code) (inline rest))))]
        [else (cons (cons (car chars) 'text) (inline (cdr chars)))]))
(define (strong-marker? chars)
  (and (not (null? chars)) (not (null? (cdr chars)))
       (equal? (car chars) (cons #\* 'text))
       (equal? (cadr chars) (cons #\* 'text))))
(define (strong chars)
  ;; Parse after inline code so literal asterisks in code stay untouched.
  (cond [(null? chars) (list)]
        [(strong-marker? chars)
         (let find ([rest (cddr chars)] [count 0])
           (cond [(null? rest) chars]
                 [(and (> count 0) (strong-marker? rest))
                  (append
                    (map (lambda (span)
                           (cons (car span) (if (equal? (cdr span) 'code) 'bold-code 'bold)))
                         (take (cddr chars) count))
                    (strong (cddr rest)))]
                 [else (find (cdr rest) (+ count 1))]))]
        [else (cons (car chars) (strong (cdr chars)))]))
(define (styled text)
  (let* ([role (cond [(starts? text "You:") (cons 4 'user)]
                     [(starts? text "Assistant:") (cons 10 'assistant)]
                     [(starts? text "Error:") (cons 6 'error)]
                     [else (cons 0 'text)])]
         [count (car role)])
    (append (paint (string->list (substring text 0 count)) (cdr role))
            (strong (inline (string->list (substring text count)))))))
(define (wrap-space? span) (member (car span) '(#\space #\tab)))
(define (word-length chars)
  (let count ([rest chars] [size 0])
    (if (or (null? rest) (wrap-space? (car rest)) (char=? (caar rest) #\newline))
        size (count (cdr rest) (+ size 1)))))
(define (trim-wrap-spaces row)
  (if (and (not (null? row)) (wrap-space? (car row)))
      (trim-wrap-spaces (cdr row)) row))
(define (rich-lines text width)
  ;; Keep styled words intact; split only tokens wider than a whole line.
  (let wrap ([chars (styled text)] [row (list)] [rows (list)] [column 0] [word-start #t])
    (cond [(null? chars) (reverse (cons (reverse (trim-wrap-spaces row)) rows))]
          [(char=? (caar chars) #\newline)
           (wrap (cdr chars) (list) (cons (reverse (trim-wrap-spaces row)) rows) 0 #t)]
          [(and (>= column (max 1 width)) (wrap-space? (car chars)))
           (wrap (cdr chars) row rows column #t)]
          [(>= column (max 1 width))
           (wrap chars (list) (cons (reverse (trim-wrap-spaces row)) rows) 0 word-start)]
          [(and word-start (> column 0) (not (wrap-space? (car chars)))
                (> (word-length chars) (- (max 1 width) column)))
           (let ([line (trim-wrap-spaces row)])
             (wrap chars (list) (if (null? line) rows (cons (reverse line) rows)) 0 #t))]
          [else (wrap (cdr chars) (cons (car chars) row) rows (+ column 1) (wrap-space? (car chars)))])))
(define (style kind)
  (let ([base (ui.theme-scope-ref
                (cond [(equal? kind 'user) "constant"]
                      [(equal? kind 'assistant) "function"]
                      [(member kind '(code bold-code)) "markup.raw"]
                      [(equal? kind 'error) "error"]
                      [else "ui.text"]))])
    (cond [(member kind '(code bold-code))
           (let* ([background (ui.style->bg (ui.theme-scope-ref "ui.popup"))]
                  [code (if (ui.Color? background) (ui.style-bg base background) base)])
             (if (equal? kind 'bold-code) (ui.style-with-bold code) code))]
          [(member kind '(user assistant error bold)) (ui.style-with-bold base)]
          [else base])))
(define (rich-draw frame x y line)
  ;; Coalesce spans before crossing the native API boundary.
  (let loop ([rest line] [chars (list)] [kind #f] [column x])
    (cond [(null? rest)
           (unless (null? chars)
             (ui.frame-set-string! frame column y (list->string (reverse chars)) (style kind)))]
          [(or (not kind) (equal? kind (cdar rest)))
           (loop (cdr rest) (cons (caar rest) chars) (cdar rest) column)]
          [else
           (ui.frame-set-string! frame column y (list->string (reverse chars)) (style kind))
           (loop rest (list) #f (+ column (length chars)))])))
