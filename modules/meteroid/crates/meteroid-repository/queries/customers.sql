--: Customer(id, name, alias?, email?, billing_config?, invoicing_email?, phone?, archived_at?, created_at?, billing_address?, shipping_address?)
--: CustomerList(alias?, email?)
--! create_customer (id, name, email?, alias?, tenant_id, created_by): (alias?, email?)
INSERT INTO customer (id, name, alias, email, tenant_id, created_by, billing_config)
VALUES (:id,
        :name,
        :alias,
        :email,
        :tenant_id,
        :created_by,
        :billing_config)
RETURNING id, name, email, alias;

--! list_customers (search?) : CustomerList
SELECT id,
       name,
       email,
       alias,
       COUNT(*) OVER () AS total_count
FROM customer
WHERE tenant_id = :tenant_id
  AND (
    :search :: TEXT IS NULL
        OR name ILIKE '%' || :search || '%'
        OR alias ILIKE '%' || :search || '%'
    )
ORDER BY CASE
             WHEN :order_by = 'DATE_DESC' THEN id
             END DESC,
         CASE
             WHEN :order_by = 'DATE_ASC' THEN id
             END ASC,
         CASE
             WHEN :order_by = 'NAME_DESC' THEN name
             END DESC,
         CASE
             WHEN :order_by = 'NAME_ASC' THEN name
             END ASC
LIMIT :limit OFFSET :offset;

--! count_customers (search?)
SELECT COUNT(*) AS total_customers
FROM customer
WHERE tenant_id = :tenant_id
  AND (
    :search :: TEXT IS NULL
        OR to_tsvector('english', name || ' ' || alias) @@ to_tsquery('english', :search)
    );

--! get_customer_by_id (id) : Customer
SELECT id,
       name,
       alias,
       billing_config,
       email,
       invoicing_email,
       phone,
       balance_value_cents,
       balance_currency,
       archived_at,
       created_at,
       billing_address,
       shipping_address
FROM customer
WHERE id = :id;


--! get_customer_by_alias (tenant_id, alias) : Customer
SELECT id,
       name,
       alias,
       billing_config,
       email,
       invoicing_email,
       phone,
       balance_value_cents,
       balance_currency,
       archived_at,
       created_at,
       billing_address,
       shipping_address
FROM customer
WHERE tenant_id = :tenant_id
  AND alias = :alias;


--! get_customer_ids_by_alias
SELECT id,
       alias
FROM customer
WHERE tenant_id = :tenant_id
  AND alias = ANY (:aliases);

--! patch_customer(alias?, email?, invoicing_email?, phone?, billing_address?, shipping_address?)
UPDATE customer
SET
    name = :name,
    alias = :alias,
    email = :email,
    invoicing_email = :invoicing_email,
    phone = :phone,
    balance_value_cents = :balance_value_cents,
    balance_currency = :balance_currency,
    billing_address = :billing_address,
    shipping_address = :shipping_address
WHERE id = :id;        